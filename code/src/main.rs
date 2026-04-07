// This is mini-power main program for ESP32-C3-WROOM.
// SPDX-License-Identifier: MIT
// Copyright (c) 2025-2026 Hiroshi Nakajima

use std::{thread, time::Duration};
use esp_idf_hal::{gpio::*, prelude::*, i2c};
use esp_idf_hal::peripherals::Peripherals;
use log::*;
use std::time::SystemTime;
use esp_idf_svc::sntp::{EspSntp, SyncStatus, SntpConf, OperatingMode, SyncMode};
use esp_idf_svc::wifi::EspWifi;
use esp_idf_svc::nvs::*;
use chrono::{DateTime, Utc};

mod displayctl;
mod currentlogs;
mod wifi;
mod transfer;
mod usbpd;
mod syslogger;  // Add the syslogger module
mod keyevent;
mod httpserver;

use displayctl::{DisplayPanel, LoggingStatus, WifiStatus};
use currentlogs::{CurrentRecord, CurrentLog};
use transfer::{Transfer, ServerInfo};
use usbpd::{AP33772S, PDVoltage};
use keyevent::{KeySwitch, KeyEvent};
use httpserver::PowerControlState;

#[toml_cfg::toml_config]
pub struct Config {
    #[default("")]
    wifi_ssid: &'static str,
    #[default("")]
    wifi_psk: &'static str,
    #[default("")]
    influxdb_server: &'static str,
    #[default("11.0")]
    max_current_limit: &'static str,
    #[default("110.0")]
    max_power_limit: &'static str,
    #[default("75.0")]
    max_temperature: &'static str,
    #[default("")]
    influxdb_api_key: &'static str,
    #[default("")]
    influxdb_api: &'static str,
    #[default("")]
    influxdb_measurement: &'static str,
    #[default("")]
    influxdb_tag: &'static str,
    #[default("")]
    syslog_server: &'static str,
    #[default("")]
    syslog_enable: &'static str,
}

// NVS key for storing the last voltage setting
const NVS_NAMESPACE: &str = "mini_power_unit";
const VOLTAGE_KEY: &str = "last_voltage";

const RECORD_MAX: usize = 127;

// Function to save voltage setting to NVS
fn save_voltage_to_nvs(voltage: f32) -> anyhow::Result<()> {
    let nvs_default_partition = EspDefaultNvsPartition::take()?;
    let mut nvs = EspNvs::new(nvs_default_partition, NVS_NAMESPACE, true)?;
    
    // Convert f32 to bytes for storage
    let voltage_bytes = voltage.to_le_bytes();
    nvs.set_blob(VOLTAGE_KEY, &voltage_bytes)?;
    info!("Voltage {:.3}V saved to NVS", voltage);
    Ok(())
}

// Function to load voltage setting from NVS
fn load_voltage_from_nvs() -> anyhow::Result<f32> {
    let nvs_default_partition = EspDefaultNvsPartition::take()?;
    let nvs = EspNvs::new(nvs_default_partition, NVS_NAMESPACE, false)?;
    
    let mut voltage_bytes = [0u8; 4];
    match nvs.get_blob(VOLTAGE_KEY, &mut voltage_bytes) {
        Ok(Some(_)) => {
            let voltage = f32::from_le_bytes(voltage_bytes);
            info!("Voltage {:.3}V loaded from NVS", voltage);
            Ok(voltage)
        },
        Ok(None) => {
            info!("No voltage setting found in NVS, using default 0.0V");
            Ok(0.0)
        },
        Err(e) => {
            info!("Failed to read voltage from NVS: {:?}, using default 0.0V", e);
            Ok(0.0)
        }
    }
}

fn main() -> anyhow::Result<()> {
    esp_idf_sys::link_patches();
    
    // Initialize the default ESP logger only if syslog is disabled
    // If syslog is enabled, we'll initialize the syslog logger later
    if CONFIG.syslog_enable != "true" {
        esp_idf_svc::log::EspLogger::initialize_default();
        // Set log level to INFO to ensure info!() messages are displayed
        log::set_max_level(log::LevelFilter::Info);
    }
    
    // Peripherals Initialize
    let peripherals = Peripherals::take().unwrap();
    // Initialize nvs
    unsafe {
        esp_idf_sys::nvs_flash_init();
    }

    // Log startup message
    println!("MiniPowerUnit application started (println)");
    info!("MiniPowerUnit application started (info)");
    
    let max_power_limit = CONFIG.max_power_limit.parse::<f32>().unwrap();
    let max_temperature = CONFIG.max_temperature.parse::<f32>().unwrap();
    let max_current_limit = CONFIG.max_current_limit.parse::<f32>().unwrap();
    println!("[Config Limit] Current: {}A  Power: {}W  Temperature: {}°C", max_current_limit, max_power_limit, max_temperature);
    info!("[Config Limit] Current: {}A  Power: {}W  Temperature: {}°C", max_current_limit, max_power_limit, max_temperature);
    let server_info = ServerInfo::new(CONFIG.influxdb_server.to_string(), 
        CONFIG.influxdb_api_key.to_string(),
        CONFIG.influxdb_api.to_string(),
        CONFIG.influxdb_measurement.to_string(),
        CONFIG.influxdb_tag.to_string());

    // GPIO21 as output for enabling AP33772S
    // let mut ap_enable_pin = PinDriver::output(peripherals.pins.gpio21)?;
    // ap_enable_pin.set_low()?; // Disable AP33772S

    // Current/Voltage
    let i2c = peripherals.i2c0;
    let scl = peripherals.pins.gpio7;
    let sda = peripherals.pins.gpio8;
    let config = i2c::I2cConfig::new().baudrate(100.kHz().into());
    let i2c_driver = i2c::I2cDriver::new(i2c, sda, scl, &config)?;

    // Clone the I2C driver for shared use (using Arc and Mutex for thread safety)
    use std::sync::{Arc, Mutex};
    let shared_i2c = Arc::new(Mutex::new(i2c_driver));
    
    // Create display with shared I2C
    let mut dp = DisplayPanel::new();
    let display_i2c = shared_i2c.clone();
    dp.start(display_i2c);

    // Use the shared I2C for INA sensor
    let ap33772s_i2c = shared_i2c.clone();

    // read config
    let mut ap33772s = AP33772S::new();
    {
        let mut i2c_driver = ap33772s_i2c.lock().unwrap();
        match ap33772s.init(&mut *i2c_driver) {
            Ok(()) => {
                info!("AP33772S initialized successfully");
            },
            Err(e) => {
                return Err(anyhow::anyhow!("Failed to initialize AP33772S: {:?}", e));
            }
        }

        // Configure protection features: UVP=true, OVP=true, OCP=true, OTP=false, DR=false
        match ap33772s.configure_protections(&mut *i2c_driver, true, true, true, false, false) {
            Ok(()) => {
                info!("AP33772S protections configured successfully");
            },
            Err(e) => {
                warn!("Failed to configure AP33772S protections: {:?}", e);
            }
        }
        match ap33772s.get_status(&mut *i2c_driver) {
            Ok(status) => {
                // For debugging purposes, log status occasionally
                // Not implemented: NTC thermistor
                info!(
                    "PD Status: Voltage={}mV, Current={}mA, Temp={}°C, PDP={}W",
                    status.voltage_mv,
                    status.current_ma,
                    status.temperature,
                    status.pdp_limit_w
                );
            },
            Err(e) => {
                info!("Failed to read AP33772S status: {:?}", e);
            }
        }
        let _ = ap33772s.request_voltage(&mut *i2c_driver, PDVoltage::V5);
        ap33772s.force_vout_off(&mut *i2c_driver).unwrap();
    }

    // Get PDO limits from connected source
    let pdo_min_voltage = 5.0;
    info!("PDO Min Voltage = {:.2}V", pdo_min_voltage);
    let (mut pdo_max_voltage, pdo_max_current) = ap33772s.get_pdo_limits();
    info!("PDO Limits: Max Voltage = {:.2}V, Max Current = {:.3}A", pdo_max_voltage, pdo_max_current);
    
    // Display PDO list on screen (each page for 5 seconds)
    let pdo_list = ap33772s.get_pdo_list();
    if pdo_list.len() > 0 {
        DisplayPanel::show_pdo_list(shared_i2c.clone(), pdo_list);
        info!("PDO display complete, resuming normal operation");
        
        // Set PDO range information for the voltage bar display
        let pdo_display_list: Vec<displayctl::PDODisplayInfo> = pdo_list.iter().map(|pdo| {
            displayctl::PDODisplayInfo {
                voltage_mv: pdo.voltage_mv,
                is_fixed: pdo.is_fixed,
            }
        }).collect();
        dp.set_pdo_range(pdo_display_list, pdo_min_voltage, pdo_max_voltage);
    }
    
    // Apply the more restrictive limit between config and PDO
    let mut effective_max_current = if pdo_max_current < max_current_limit { pdo_max_current } else { max_current_limit };
    info!("Effective Current Limit: {:.3}A (Config: {:.3}A, PDO: {:.3}A)", 
          effective_max_current, max_current_limit, pdo_max_current);
    println!("[Effective Limits] Voltage: {:.2}V  Current: {:.3}A", pdo_max_voltage, effective_max_current);
    if pdo_max_voltage <= 0.0 {
        info!("Warning: Connected source is not PD powered.");
        dp.set_message("Warning: No PD Power".to_string(), true, 0);
        pdo_max_voltage = 5.0; // Set a default voltage to allow operation
        effective_max_current = 3.0; // Set a default current limit
    }

    // Temperature Logs
    let mut clogs = CurrentRecord::new();

    // Initialize logging for early debugging
    let mut wifi_enable : bool;
    let mut wifi_dev = wifi::wifi_connect(peripherals.modem, CONFIG.wifi_ssid, CONFIG.wifi_psk);

    if CONFIG.syslog_enable == "true" {
        // Initialize syslog logger to replace the default ESP logger
        println!("Initializing syslog logger...");
        thread::sleep(Duration::from_secs(5));
        
        match syslogger::init_logger(CONFIG.syslog_server, CONFIG.syslog_enable) {
            Ok(_) => {
                // Set log level for syslog
                log::set_max_level(log::LevelFilter::Info);
                println!("Syslog logger initialized successfully");
                info!("Syslog logger initialized successfully");
            },
            Err(e) => {
                // Fallback to ESP logger if syslog fails
                println!("Failed to initialize syslog logger: {:?}, using ESP logger instead", e);
                esp_idf_svc::log::EspLogger::initialize_default();
                log::set_max_level(log::LevelFilter::Info);
                info!("Failed to initialize syslog logger: {:?}, using ESP logger instead", e);
            }
        }
    } else {
        // syslog_enable is false, continue using default ESP console logger
        info!("Using default ESP console logger (syslog disabled)");
    }
    
    // NTP Server
    let sntp_conf = SntpConf {
        servers: ["time.aws.com",
                    "time.google.com",
                    "time.cloudflare.com",
                    "ntp.nict.jp"],
        operating_mode: OperatingMode::Poll,
        sync_mode: SyncMode::Immediate,
    };
    let ntp = EspSntp::new(&sntp_conf).unwrap();

    // NTP Sync
    // let now = SystemTime::now();
    // if now.duration_since(UNIX_EPOCH).unwrap().as_millis() < 1700000000 {
    info!("NTP Sync Start..");

    // wait for sync
    let mut sync_count = 0;
    while ntp.get_sync_status() != SyncStatus::Completed {
        sync_count += 1;
        if sync_count > 1000 {
            info!("NTP Sync Timeout");
            break;
        }
        thread::sleep(Duration::from_millis(10));
    }
    let now = SystemTime::now();
    let dt_now : DateTime<Utc> = now.into();
    let formatted = format!("{}", dt_now.format("%Y-%m-%d %H:%M:%S"));
    info!("NTP Sync Completed: {}", formatted);
    
    // Get and display IP address
    if let Ok(ref wifi) = wifi_dev {
        if let Ok(ip_info) = wifi.sta_netif().get_ip_info() {
            let ip_str = format!("{}", ip_info.ip);
            info!("IP Address: {}", ip_str);
            dp.set_ip_address(ip_str);
        }
    }
    
    // Create HTTP server state
    let http_state = PowerControlState::new(pdo_min_voltage, pdo_max_voltage);
    
    // Set PDO list in HTTP state
    {
        let pdo_list = ap33772s.get_pdo_list();
        let pdo_simple_list: Vec<httpserver::PDOInfoSimple> = pdo_list.iter().map(|pdo| {
            httpserver::PDOInfoSimple {
                pdo_index: pdo.pdo_index,
                voltage_mv: pdo.voltage_mv,
                current_ma: pdo.current_ma,
                max_power_mw: pdo.max_power_mw,
                is_fixed: pdo.is_fixed,
            }
        }).collect();
        http_state.set_pdo_list(pdo_simple_list);
        info!("PDO list set in HTTP state: {} PDOs", pdo_list.len());
    }
    
    // Start HTTP server
    let _http_server = match httpserver::start_http_server(http_state.clone()) {
        Ok(server) => {
            info!("HTTP server started successfully");
            Some(server)
        },
        Err(e) => {
            info!("Failed to start HTTP server: {:?}", e);
            None
        }
    };
        
    let mut txd =  Transfer::new(server_info);
    txd.start()?;

    // Key Event Handler
    let mut keyswitch = KeySwitch::new();
    let gpio_up = Box::new(PinDriver::input(peripherals.pins.gpio10)?);
    let gpio_down = Box::new(PinDriver::input(peripherals.pins.gpio20)?);
    let gpio_center = Box::new(PinDriver::input(peripherals.pins.gpio21)?);
    keyswitch.start(gpio_up, gpio_down, gpio_center);
    
    // loop
    let mut measurement_count : u32 = 0;
    let mut logging_start: bool = false;
    let mut load_start = false;
    let mut previous_load_start = false;
    
    // Load last voltage setting from NVS
    let mut set_output_voltage = match load_voltage_from_nvs() {
        Ok(voltage) => {
            // Ensure voltage is within PDO limits
            if voltage > pdo_max_voltage {
                info!("Loaded voltage {:.3}V exceeds PDO limit {:.3}V, using limit", voltage, pdo_max_voltage);
                pdo_max_voltage
            } else if voltage < pdo_min_voltage {
                info!("Loaded voltage {:.3}V below PDO min limit {:.3}V, using min limit", voltage, pdo_min_voltage);
                pdo_min_voltage
            } else {
                voltage
            }
        },
        Err(e) => {
            info!("Failed to load voltage from NVS: {:?}, using 0.0V", e);
            pdo_min_voltage
        }
    };

    // get calibration offset when this starts
    let calibration_offset_voltage: f32 = 0.0;
    let calibration_offset_current: f32 = 0.0;
    // {
    //     let mut i2c_driver = ap33772s_i2c.lock().unwrap();
    //     match ap33772s.get_status(&mut *i2c_driver) {
    //         Ok(status) => {
    //             // calibration_offset_voltage = status.voltage_mv as f32;
    //             calibration_offset_current = status.current_ma as f32;
    //             info!("Calibration offset determined: {}mV, {}mA", calibration_offset_voltage, calibration_offset_current);
    //         },
    //         Err(e) => {
    //             info!("Failed to read AP33772S status for calibration: {:?}, assuming 0.0V offset", e);
    //             calibration_offset_voltage = 0.0;
    //             calibration_offset_current = 0.0;
    //         }
    //     }
    // }


    info!("Initial voltage setting: {:.3}V", set_output_voltage);
    let mut previous_set_output_voltage = set_output_voltage;
    let mut confirmed_set_voltage = set_output_voltage;
    let mut last_set_output_voltage = SystemTime::now();
    // Set initial voltage display
    dp.set_output_voltage(set_output_voltage);
    // Initialize HTTP state with NVS-loaded target so web UI shows saved value
    http_state.set_target_voltage(set_output_voltage);
    http_state.set_output_enabled(load_start);
    loop {
        thread::sleep(Duration::from_millis(100));
        measurement_count += 1;
        
        // Check HTTP server state and apply changes
        let http_target_voltage = http_state.get_target_voltage();
        let http_output_enabled = http_state.get_output_enabled();
        
        // // Debug log to check http_target_voltage updates
        // if measurement_count % 50 == 0 { // Every 5 seconds
        //     info!("Debug: http_target_voltage={:.3}V, set_output_voltage={:.3}V, diff={:.3}V", 
        //           http_target_voltage, set_output_voltage, (http_target_voltage - set_output_voltage).abs());
        // }
        
        // If HTTP server changed the target voltage, update it
        if (http_target_voltage - set_output_voltage).abs() > 0.05 {
            set_output_voltage = http_target_voltage;
            last_set_output_voltage = SystemTime::now();
            dp.set_output_voltage(set_output_voltage);
            info!("HTTP: Voltage changed to {:.2}V", set_output_voltage);
        }
        
        // If HTTP server changed the output state, update it
        if http_output_enabled != load_start {
            load_start = http_output_enabled;
            info!("HTTP: Output {} via web interface", if load_start { "enabled" } else { "disabled" });
        }
        
        // get key event
        let key_event = keyswitch.get_key_event_and_clear();
        if key_event.is_empty() == false {
            dp.set_message("".to_string(), false, 0);
        }
        for event in key_event {
            match event {
                KeyEvent::UpKeyDown => {
                    set_output_voltage += 0.1;
                    if set_output_voltage > pdo_max_voltage {
                        set_output_voltage = pdo_max_voltage;
                    }
                    last_set_output_voltage = SystemTime::now();
                    dp.set_output_voltage(set_output_voltage);
                    http_state.set_target_voltage(set_output_voltage);
                },
                KeyEvent::DownKeyDown => {
                    set_output_voltage -= 0.1;
                    if set_output_voltage < pdo_min_voltage {
                        set_output_voltage = pdo_min_voltage;
                    }
                    last_set_output_voltage = SystemTime::now();
                    dp.set_output_voltage(set_output_voltage);
                    http_state.set_target_voltage(set_output_voltage);
                },
                KeyEvent::CenterKeyDown => {
                    // Toggle load_start when Center key is pressed
                    load_start = !load_start;
                    http_state.set_output_enabled(load_start);
                    info!("Center key pressed! load_start toggled to: {}", load_start);
                },
                KeyEvent::UpDownLongPress => {
                },
                _ => {},
            }
        }

        let rssi = wifi::get_rssi();
        if rssi == 0 {
            wifi_enable = false;
            if measurement_count % 100 == 0 {
                wifi_reconnect(&mut wifi_dev.as_mut().unwrap());
            }
        }
        else {
            wifi_enable = true;
        }

        if wifi_enable == false {
            dp.set_wifi_status(WifiStatus::Disconnected);
        }
        else {
            dp.set_wifi_rssi(rssi);
            dp.set_wifi_status(WifiStatus::Connected);
        }

        // Normal mode: load_start controls the output as before
        if load_start == true {
            // pid.set_setpoint(set_output_voltage);
            let diff_setpoint = set_output_voltage - previous_set_output_voltage;
            if (diff_setpoint.abs() >= 0.05 && last_set_output_voltage.elapsed().unwrap().as_secs() >= 3) || (previous_load_start == false)  {
                // Set USB PD Voltage
                confirmed_set_voltage = set_output_voltage;
                info!("Changing USB PD Voltage to {:.2}V from {:.2}V", set_output_voltage, previous_set_output_voltage);
                dp.set_output_voltage_changing(true);
                {
                    let mut i2c_driver = ap33772s_i2c.lock().unwrap();
                    let actual_voltage = match usbpd_control(&mut ap33772s, &mut *i2c_driver, set_output_voltage, 0.0) {
                        Ok(v) => {
                            info!("USB PD Voltage set to {:.2}V successfully (requested: {:.2}V)", v, set_output_voltage);
                            v
                        },
                        Err(e) => {
                            info!("Failed to set USB PD Voltage to {:.2}V: {:?}", set_output_voltage, e);
                            dp.set_message("Failed to set Voltage".to_string(), true, 3000);
                            load_start = false;
                            http_state.set_output_enabled(false);
                            continue;
                        }
                    };
                    
                    // Update set_output_voltage with actual voltage (for Fixed PDO fallback)
                    if (actual_voltage - set_output_voltage).abs() > 0.05 {
                        info!("Voltage fallback: requested {:.2}V, using Fixed PDO {:.2}V", set_output_voltage, actual_voltage);
                        set_output_voltage = actual_voltage;
                        http_state.set_target_voltage(set_output_voltage);
                        confirmed_set_voltage = set_output_voltage;
                    }
                }
                dp.set_output_voltage(set_output_voltage);
                
                previous_set_output_voltage = set_output_voltage;
                if let Err(e) = save_voltage_to_nvs(set_output_voltage) {
                    info!("Failed to save voltage to NVS: {:?}", e);
                }
                if previous_load_start == false {
                    let mut i2c_driver = ap33772s_i2c.lock().unwrap();
                    ap33772s.force_vout_on(&mut *i2c_driver).unwrap();
                    // ap_enable_pin.set_high().unwrap(); // Enable Output
                    info!("Load started, applying voltage {:.2}V", set_output_voltage);
                    logging_start = true;
                }
                previous_load_start = load_start;
            }
            dp.set_current_status(LoggingStatus::Start);
            }
        else {
            logging_start = false;
            if previous_load_start == true {
                let mut i2c_driver = ap33772s_i2c.lock().unwrap();
                ap33772s.force_vout_off(&mut *i2c_driver).unwrap();
                // ap_enable_pin.set_low()?; // Disable Output
                info!("Load stopped.");
            }
            previous_load_start = load_start;
            dp.set_current_status(LoggingStatus::Stop);
        }

        // Read Current/Voltage
        let mut data = CurrentLog::default();
        // Timestamp
        let now = SystemTime::now();
        // set clock in ns
        data.clock = now.duration_since(SystemTime::UNIX_EPOCH).unwrap().as_nanos();

        // get current and power from AP33772S
        // only when load is started, because reading current when load is off may give invalid values.
        if load_start == true {
            let mut i2c_driver = ap33772s_i2c.lock().unwrap();
            match ap33772s.get_status(&mut *i2c_driver) {
                Ok(status) => {
                    data.voltage = (status.voltage_mv as f32 - calibration_offset_voltage) / 1000.0; // mV to V
                    data.current = (status.current_ma as f32 - calibration_offset_current) / 1000.0; // mA to A
                    data.power = data.voltage * data.current; // Calculate power as V * I
                    data.temp = status.temperature as f32;  // °C
                },
                Err(e) => {
                    info!("Failed to read AP33772S status: {:?}", e);
                    data.voltage = 0.0;
                    data.current = 0.0;
                    data.power = 0.0;
                    data.temp = 0.0;
                }
            }
        }
        // info!("V={:.3}V I={:.3}A P={:.1}W T={:.1}°C", 
        //       data.voltage, data.current, data.power, data.temp);
        dp.set_voltage(data.voltage, data.current, data.power);
        
        // Update HTTP server state with current measurements
        http_state.update_measurements(data.voltage, data.current, data.power);
        
        if load_start == true {
            // Check voltage overshoot (>110% of setpoint)
            let voltage_overshoot_threshold = confirmed_set_voltage * 1.10;
            if data.voltage > voltage_overshoot_threshold && confirmed_set_voltage > 0.0 {
                info!("Voltage overshoot detected: {:.3}V > {:.3}V (110% of {:.3}V)", 
                      data.voltage, voltage_overshoot_threshold, confirmed_set_voltage);
                dp.set_message("Voltage Overshoot".to_string(), true, 3000);
                load_start = false;
            }
            // Current and Power Limit
            if data.current > effective_max_current {
                info!("Current Limit Over: {:.3}A (PDO Limited)", data.current);
                dp.set_message(format!("Current OV {:.3}A", data.current), true, 3000);
                load_start = false;
            }
            if data.power > max_power_limit {
                info!("Power Limit Over: {:.1}W", data.power);
                dp.set_message(format!("Power OV {:.1}W", data.power), true, 3000);
                load_start = false;
            }
            if load_start == false {
                http_state.set_output_enabled(false);
            }
        }

        if logging_start {
            clogs.record(data);
        }
        let current_record = clogs.get_size();
        if current_record >= RECORD_MAX {
            logging_start = false;  // Auto stop logging if buffer is full.
        }
        dp.set_buffer_watermark((current_record as u32) * 100 / RECORD_MAX as u32);

        if wifi_enable == true && current_record > 0 {
            let logs = clogs.get_all_data();
            let txcount = txd.set_transfer_data(logs);
            if txcount > 0 {
                clogs.remove_data(txcount);
            }
        }
    }
}


fn usbpd_control(ap33772s: &mut AP33772S, i2cdrv: &mut i2c::I2cDriver, voltage: f32, pd_config_offset: f32) -> anyhow::Result<f32> {
    // USB PD Control - returns actual voltage set in V
    ap33772_usbpd_control(ap33772s, i2cdrv, voltage, pd_config_offset)
}

fn ap33772_usbpd_control(ap33772s: &mut AP33772S, i2cdrv: &mut i2c::I2cDriver, voltage: f32, pd_config_offset: f32) -> anyhow::Result<f32> {
    // USB PD Control
    // Set voltage
    if voltage <= 0.0 {
        // Disable Output
        let _ = ap33772s.request_voltage(i2cdrv, PDVoltage::V5);
        // ap33772s.force_vout_off(i2cdrv).unwrap();
        return Err(anyhow::anyhow!("Voltage is zero or negative"));
    }
    // ap33772s.set_vout_auto_control(i2cdrv).unwrap();
    let mut max_current_limit = 5000; // 5A
    let mut req_voltage = voltage + pd_config_offset;
    let available_voltage = ap33772s.get_max_voltage() as f32 / 1000.0;
    if req_voltage > available_voltage {
        info!("Requested voltage exceeds available voltage: {} > {}", req_voltage, available_voltage);
        req_voltage = available_voltage;
    }
    let pd_voltage = (req_voltage * 1000.0) as u16;
    // Try to request custom voltage PPS APDO
    match ap33772s.request_custom_voltage(i2cdrv, pd_voltage, max_current_limit) {
        Ok(actual_voltage_mv) => {
            let actual_voltage_v = actual_voltage_mv as f32 / 1000.0;
            info!("USB PD voltage set successfully: requested {:.3}V, actual {:.3}V", req_voltage, actual_voltage_v);
            return Ok(actual_voltage_v);
        },
        Err(e) => {
            info!("Failed to request voltage: {:?}", e);
        }
    }
    // try to request maximum current to be 3A
    max_current_limit = 3000;
    // try to request custom voltage PPS APDO
    match ap33772s.request_custom_voltage(i2cdrv, pd_voltage, max_current_limit) {
        Ok(actual_voltage_mv) => {
            let actual_voltage_v = actual_voltage_mv as f32 / 1000.0;
            info!("USB PD voltage set successfully: requested {:.3}V, actual {:.3}V", req_voltage, actual_voltage_v);
            return Ok(actual_voltage_v);
        },
        Err(e) => {
            info!("Failed to request voltage: {:?}", e);
        }
    }
    // Fallback to 5V
    // This unit needs to power on 5V.
    match ap33772s.request_voltage(i2cdrv, PDVoltage::V5) {
        Ok(()) => {
            info!("Fallback to 5V successful");
            return Ok(5.0);
        },
        Err(e) => {
            info!("Failed to request 5V: {:?}", e);
        }
    }
    // if the power source is not PD and the requested voltage is 5V, consider it success.
    if voltage == 5.0 {
        return Ok(5.0);
    }
    Err(anyhow::anyhow!("Failed to set requested voltage"))
}

fn wifi_reconnect(wifi_dev: &mut EspWifi) -> bool{
    unsafe {
        esp_idf_sys::esp_wifi_start();
    }
    match wifi_dev.connect() {
        Ok(_) => { info!("Wifi connecting requested."); true},
        Err(ref e) => { info!("{:?}", e); false }
    }
}
