// This is mini-power display control module for ESP32-C3-WROOM.
// SPDX-License-Identifier: MIT
// Copyright (c) 2025-2026 Hiroshi Nakajima

use log::*;
use std::{thread, time::Duration, sync::Arc, sync::Mutex, time::SystemTime};
use esp_idf_hal::i2c;
use ssd1306::{I2CDisplayInterface, prelude::*, Ssd1306};
use embedded_graphics::{
    mono_font::{ascii::{FONT_10X20, FONT_5X8, FONT_6X10}, MonoTextStyle, MonoTextStyleBuilder},
    pixelcolor::BinaryColor,
    text::{Text},
    geometry::{Point, Size},
    prelude::*,
    image::Image,
    primitives::{Rectangle, PrimitiveStyle, Line},
};
use tinybmp::Bmp;

// PDO information for display
#[derive(Clone)]
pub struct PDODisplayInfo {
    pub voltage_mv: u16,
    pub is_fixed: bool,
}

pub enum LoggingStatus {
    Start,
    Stop,
}

#[allow(dead_code)]
pub enum WifiStatus {
    Disconnected,
    Connecting,
    Connected,
}

struct DisplayText {
    voltage: f32,
    current: f32,
    power: f32,
    wifi_rssi: i32,
    message: String,
    message_enable: bool,
    message_timer: SystemTime,
    message_timeout: u32,
    status: LoggingStatus,
    wifi: WifiStatus,
    buffer_water_mark: u32,
    channel: u32,
    output_voltage: f32,
    output_voltage_changing: bool,
    // voltage_range: u8,  // 0=mV, 1=V
    // current_range: u8,  // 0=mA, 1=A
    power_range: u8,    // 0=mW, 1=W
    ip_address: String,
    pdo_list: Vec<PDODisplayInfo>,
    pdo_min_voltage: f32,
    pdo_max_voltage: f32,
}

pub struct DisplayPanel {
    txt: Arc<Mutex<DisplayText>>
}

#[allow(dead_code)]
impl DisplayPanel {

    pub fn new() -> DisplayPanel {
        DisplayPanel { txt: Arc::new(Mutex::new(
            DisplayText {voltage: 0.0,
                         message: "".to_string(),
                         message_enable: false,
                         message_timer: SystemTime::now(),
                         message_timeout: 0,
                         current: 0.0,
                         power: 0.0,
                         wifi_rssi: 0,
                         status: LoggingStatus::Stop,
                         wifi: WifiStatus::Disconnected,
                         buffer_water_mark: 0,
                         channel: 1, // Default channel
                         output_voltage: 0.0,
                        //  voltage_range: 1, // Default to V
                        //  current_range: 1, // Default to A
                         power_range: 1,   // Default to W
                         output_voltage_changing: false,
                         ip_address: "".to_string(),
                         pdo_list: Vec::new(),
                         pdo_min_voltage: 5.0,
                         pdo_max_voltage: 21.0,
                     })) }
    }

    pub fn start(&mut self, shared_i2c: Arc<Mutex<i2c::I2cDriver<'static>>>)
    {
        let txt = self.txt.clone();
        let _th = thread::spawn(move || {
            info!("Start Display Thread.");
            
            // Create a simple wrapper that implements the required traits for SSD1306
            struct I2CWrapper {
                driver: Arc<Mutex<i2c::I2cDriver<'static>>>,
            }
            
            impl embedded_hal_0_2::blocking::i2c::Write for I2CWrapper {
                type Error = ();
                
                fn write(&mut self, address: u8, bytes: &[u8]) -> Result<(), Self::Error> {
                    let mut driver = self.driver.lock().unwrap();
                    driver.write(address, bytes, esp_idf_hal::delay::BLOCK).map_err(|_| ())
                }
            }
            
            let wrapper = I2CWrapper { driver: shared_i2c };
            let interface = I2CDisplayInterface::new(wrapper);
            let mut display = Ssd1306::new(interface, 
                DisplaySize128x64,
                ssd1306::prelude::DisplayRotation::Rotate180)
                .into_buffered_graphics_mode();
                
            if let Err(e) = display.init() {
                info!("Display init failed: {:?}", e);
                return;
            }
            
            let style_large = MonoTextStyle::new(&FONT_10X20, BinaryColor::On);
            // let style_large_inv = MonoTextStyleBuilder::new()
            //     .font(&FONT_10X20)
            //     .text_color(BinaryColor::Off)
            //     .background_color(BinaryColor::On)
            //     .build();
            let style_small = MonoTextStyle::new(&FONT_5X8, BinaryColor::On);
            let style_middle = MonoTextStyle::new(&FONT_6X10, BinaryColor::On);
            let style_middle_inv = MonoTextStyleBuilder::new()
                .font(&FONT_6X10)
                .text_color(BinaryColor::Off)
                .background_color(BinaryColor::On)
                .build();
            
            // Wifi BMP
            let wifi_x = 78;
            let wifibmp0 = Bmp::from_slice(include_bytes!("./img/wifi-0.bmp")).unwrap();
            let wifi_img0: Image<Bmp<BinaryColor>> = Image::new(&wifibmp0, Point::new(wifi_x,0));
            let wifibmp1 = Bmp::from_slice(include_bytes!("./img/wifi-1.bmp")).unwrap();
            let wifi_img1: Image<Bmp<BinaryColor>> = Image::new(&wifibmp1, Point::new(wifi_x,0));
            let wifibmp2 = Bmp::from_slice(include_bytes!("./img/wifi-2.bmp")).unwrap();
            let wifi_img2: Image<Bmp<BinaryColor>> = Image::new(&wifibmp2, Point::new(wifi_x,0));
            let wifibmp3 = Bmp::from_slice(include_bytes!("./img/wifi-3.bmp")).unwrap();
            let wifi_img3: Image<Bmp<BinaryColor>> = Image::new(&wifibmp3, Point::new(wifi_x,0));
            let wifibmp4 = Bmp::from_slice(include_bytes!("./img/wifi-4.bmp")).unwrap();
            let wifi_img4: Image<Bmp<BinaryColor>> = Image::new(&wifibmp4, Point::new(wifi_x,0));

            // Battery BMP
            // let bat_x = 112;
            // let bat_y = 42;
            // let bat0 = Bmp::from_slice(include_bytes!("./img/battery-0.bmp")).unwrap();
            // let bat0_img: Image<Bmp<BinaryColor>> = Image::new(&bat0, Point::new(bat_x, bat_y));
            // let bat20 = Bmp::from_slice(include_bytes!("./img/battery-20.bmp")).unwrap();
            // let bat20_img: Image<Bmp<BinaryColor>> = Image::new(&bat20, Point::new(bat_x, bat_y));
            // let bat40 = Bmp::from_slice(include_bytes!("./img/battery-40.bmp")).unwrap();
            // let bat40_img: Image<Bmp<BinaryColor>> = Image::new(&bat40, Point::new(bat_x, bat_y));
            // let bat60 = Bmp::from_slice(include_bytes!("./img/battery-60.bmp")).unwrap();
            // let bat60_img: Image<Bmp<BinaryColor>> = Image::new(&bat60, Point::new(bat_x, bat_y));
            // let bat80 = Bmp::from_slice(include_bytes!("./img/battery-80.bmp")).unwrap();
            // let bat80_img: Image<Bmp<BinaryColor>> = Image::new(&bat80, Point::new(bat_x, bat_y));
            // let bat100 = Bmp::from_slice(include_bytes!("./img/battery-100.bmp")).unwrap();
            // let bat100_img: Image<Bmp<BinaryColor>> = Image::new(&bat100, Point::new(bat_x, bat_y));
            // let usbpwr = Bmp::from_slice(include_bytes!("./img/usb-power.bmp")).unwrap();
            // let usbpwr_img: Image<Bmp<BinaryColor>> = Image::new(&usbpwr, Point::new(bat_x, bat_y));

            // Clear display
            display.clear();
            display.flush().unwrap();
            
            let mut loopcount = 0;
            // let mut battery_level = 0;
            
            // Blink control for output voltage
            let mut blink_count = 0;
            let mut blink_state = false;
            let mut is_blinking = false;
            
            // Previous values for change detection
            let mut prev_voltage = -1.0;
            let mut prev_current = -1.0;
            let mut prev_power = -1.0;
            // let mut prev_voltage_range = 255;
            // let mut prev_current_range = 255;
            let mut prev_power_range = 255;
            let mut prev_output_voltage = -1.0;
            let mut prev_status = LoggingStatus::Stop;
            let mut prev_wifi_status = WifiStatus::Disconnected;
            let mut prev_wifi_rssi = -999;
            let mut prev_buffer_wm = 999;
            // let mut prev_battery = -1.0;
            // let mut prev_battery_level = 999;
            let mut prev_channel = 0;
            let mut prev_message = String::new();
            let mut prev_loopcount_display = 0;
            let mut prev_ip_address = String::new();
            
            loop {
                let mut lck = txt.lock().unwrap();
                loopcount += 1;
                if loopcount > 15 {
                    loopcount = 0;
                }
                
                // Handle output voltage blinking
                if lck.output_voltage_changing && !is_blinking {
                    // Start blinking sequence
                    is_blinking = true;
                    blink_count = 0;
                    blink_state = false;
                }
                
                if is_blinking {
                    // Toggle blink state every loop iteration (100ms)
                    blink_state = !blink_state;
                    if !blink_state {
                        // Count complete blink cycles (on->off->on)
                        blink_count += 1;
                    }
                    if blink_count >= 3 {
                        // Finished 3 blinks, stop blinking
                        is_blinking = false;
                        blink_state = false;
                        // Reset the changing flag
                        lck.output_voltage_changing = false;
                    }
                }
                
                if lck.message_enable {
                    display.clear();
                    if lck.message_timeout > 0 && lck.message_timer.elapsed().unwrap().as_millis() > lck.message_timeout as u128 {
                        lck.message_enable = false;
                    }
                    else {
                        Text::new(&format!("{}", lck.message), Point::new(1, 20), style_middle).draw(&mut display).unwrap();
                        display.flush().unwrap();
                    }
                    drop(lck);
                    thread::sleep(Duration::from_millis(100));
                    continue;
                }


                // Auto-range voltage display with hysteresis
                let voltage = lck.voltage;
                // let voltage_abs = voltage.abs();
                // match lck.voltage_range {
                //     0 => { // mV range
                //         if voltage_abs >= 2.0 { // 2V threshold to go up
                //             lck.voltage_range = 1;
                //         }
                //     },
                //     1 => { // V range
                //         if voltage_abs < 1.5 { // 1.5V threshold to go down
                //             lck.voltage_range = 0;
                //         }
                //     },
                //     _ => {
                //         lck.voltage_range = 1;
                //     }
                // }

                // Auto-range current display with hysteresis
                let current = lck.current;
                // let current_abs = current.abs();
                // match lck.current_range {
                //     0 => { // mA range
                //         if current_abs >= 2.0 { // 2A threshold to go up
                //             lck.current_range = 1;
                //         }
                //     },
                //     1 => { // A range
                //         if current_abs < 1.5 { // 1.5A threshold to go down
                //             lck.current_range = 0;
                //         }
                //     },
                //     _ => {
                //         lck.current_range = 1;
                //     }
                // }

                // Auto-range power display with hysteresis
                let power = lck.power;
                let power_abs = power.abs();
                match lck.power_range {
                    0 => { // mW range
                        if power_abs >= 1.0 { // 1W threshold to go up
                            lck.power_range = 1;
                        }
                    },
                    1 => { // W range
                        if power_abs < 0.5 { // 0.5W threshold to go down
                            lck.power_range = 0;
                        }
                    },
                    _ => {
                        lck.power_range = 1;
                    }
                }

                // Check if anything has changed that requires display update
                let wifi_changed = match (&lck.wifi, &prev_wifi_status) {
                    (WifiStatus::Disconnected, WifiStatus::Disconnected) => false,
                    (WifiStatus::Connecting, WifiStatus::Connecting) => loopcount != prev_loopcount_display, // Animation frames
                    (WifiStatus::Connected, WifiStatus::Connected) => lck.wifi_rssi != prev_wifi_rssi,
                    _ => true,
                };

                let status_changed = match (&lck.status, &prev_status) {
                    (LoggingStatus::Start, LoggingStatus::Start) => false,
                    (LoggingStatus::Stop, LoggingStatus::Stop) => false,
                    _ => true,
                };

                let display_needs_update = 
                    lck.voltage != prev_voltage ||
                    lck.current != prev_current ||
                    lck.power != prev_power ||
                    // lck.voltage_range != prev_voltage_range ||
                    // lck.current_range != prev_current_range ||
                    lck.power_range != prev_power_range ||
                    lck.output_voltage != prev_output_voltage ||
                    is_blinking ||  // Force update when blinking
                    status_changed ||
                    wifi_changed ||
                    lck.buffer_water_mark != prev_buffer_wm ||
                    lck.channel != prev_channel ||
                    lck.message != prev_message ||
                    lck.ip_address != prev_ip_address;

                // Only update display if something changed
                if display_needs_update {
                    display.clear();

                    // Display voltage with auto-range
                    // match lck.voltage_range {
                    //     0 => { // mV
                    //         Text::new(&format!("V:{:.0}mV", voltage * 1_000.0), Point::new(1, 15), style_large).draw(&mut display).unwrap();
                    //     },
                        // 1 => { // V
                            Text::new(&format!("V:{:.1}V", voltage), Point::new(1, 15), style_large).draw(&mut display).unwrap();
                    //     },
                    //     _ => {}
                    // }
                    
                    // Display current with auto-range
                    // match lck.current_range {
                    //     0 => { // mA
                    //         Text::new(&format!("I:{:.0}mA", current * 1_000.0), Point::new(1, 30), style_large).draw(&mut display).unwrap();
                    //     },
                    //     1 => { // A
                            Text::new(&format!("I:{:.1}A", current), Point::new(1, 30), style_large).draw(&mut display).unwrap();
                    //     },
                    //     _ => {}
                    // }

                    // setup output voltage display with blinking support
                    let output_voltage = lck.output_voltage;
                    if is_blinking && blink_state {
                        // Inverted display during blink
                        Text::new(&format!("SET:{:.1}V", output_voltage), Point::new(1, 40), style_middle_inv).draw(&mut display).unwrap();
                    } else {
                        // Normal display
                        Text::new(&format!("SET:{:.1}V", output_voltage), Point::new(1, 40), style_middle).draw(&mut display).unwrap();
                    }

                    // Display power with auto-range
                    match lck.power_range {
                        0 => { // mW
                            Text::new(&format!("P:{:.0}mW", power * 1_000.0), Point::new(62, 40), style_middle).draw(&mut display).unwrap();
                        },
                        1 => { // W
                            Text::new(&format!("P:{:.1}W", power), Point::new(62, 40), style_middle).draw(&mut display).unwrap();
                        },
                        _ => {}
                    }
                                    
                    // Display logging status
                    match lck.status {
                        LoggingStatus::Start => {
                            Text::new("OUTPUT", Point::new(1, 50), style_middle_inv).draw(&mut display).unwrap();
                        },
                        LoggingStatus::Stop => {
                            Text::new("STOPPED", Point::new(1, 50), style_middle).draw(&mut display).unwrap();
                        }
                    }
                    
                    // Display buffer watermark as bar
                    let bar_x = 1;
                    let bar_y = 55;
                    let bar_width = 60;
                    let bar_height = 5;
                    
                    // Draw outer frame
                    Rectangle::new(Point::new(bar_x, bar_y), Size::new(bar_width, bar_height))
                        .into_styled(PrimitiveStyle::with_stroke(BinaryColor::On, 1))
                        .draw(&mut display).unwrap();
                    
                    // Calculate filled width based on watermark percentage
                    let filled_width = (bar_width as u32 - 2) * lck.buffer_water_mark / 100;
                    if filled_width > 0 {
                        Rectangle::new(Point::new(bar_x + 1, bar_y + 1), Size::new(filled_width, bar_height - 2))
                            .into_styled(PrimitiveStyle::with_fill(BinaryColor::On))
                            .draw(&mut display).unwrap();
                    }
                    
                    // Display percentage text next to the bar
                    Text::new(&format!("{}%", lck.buffer_water_mark), Point::new(65, 60), style_small).draw(&mut display).unwrap();
                    
                    // Draw PDO voltage range bar on the right edge
                    let pdo_bar_x = 120;
                    let pdo_bar_y = 8;  // Start lower to leave space for top text
                    let pdo_bar_width = 3;
                    let pdo_bar_height = 48;  // Reduced height to leave space for text at top and bottom
                    
                    if !lck.pdo_list.is_empty() && lck.pdo_max_voltage > lck.pdo_min_voltage {
                        let voltage_range = lck.pdo_max_voltage - lck.pdo_min_voltage;
                        
                        // Find the maximum voltage of Variable (AVS) PDOs
                        let mut avs_max_voltage = lck.pdo_min_voltage;
                        for pdo in &lck.pdo_list {
                            if !pdo.is_fixed {  // Variable PDO (AVS)
                                let pdo_voltage_v = pdo.voltage_mv as f32 / 1000.0;
                                if pdo_voltage_v > avs_max_voltage {
                                    avs_max_voltage = pdo_voltage_v;
                                }
                            }
                        }
                        
                        // Draw outer frame (full range from min to configured max)
                        Rectangle::new(Point::new(pdo_bar_x, pdo_bar_y), Size::new(pdo_bar_width, pdo_bar_height))
                            .into_styled(PrimitiveStyle::with_stroke(BinaryColor::On, 1))
                            .draw(&mut display).unwrap();
                        
                        // Fill only the variable (AVS) range with white (from min to AVS max voltage)
                        if avs_max_voltage > lck.pdo_min_voltage {
                            // Calculate the fill height based on AVS available voltage
                            let normalized_max = (lck.pdo_max_voltage - avs_max_voltage) / voltage_range;
                            let fill_y_start = pdo_bar_y + (normalized_max * (pdo_bar_height - 2) as f32) as i32 + 1;
                            let fill_height = (pdo_bar_height as i32 - 2) - (normalized_max * (pdo_bar_height - 2) as f32) as i32;
                            
                            if fill_height > 0 {
                                Rectangle::new(
                                    Point::new(pdo_bar_x + 1, fill_y_start), 
                                    Size::new(pdo_bar_width - 2, fill_height as u32)
                                )
                                .into_styled(PrimitiveStyle::with_fill(BinaryColor::On))
                                .draw(&mut display).unwrap();
                            }
                        }
                        
                        // Draw horizontal lines for Fixed PDOs extending outside the bar
                        for pdo in &lck.pdo_list {
                            if pdo.is_fixed {
                                let pdo_voltage_v = pdo.voltage_mv as f32 / 1000.0;
                                if pdo_voltage_v >= lck.pdo_min_voltage && pdo_voltage_v <= lck.pdo_max_voltage {
                                    // Calculate Y position (inverted: top=max, bottom=min)
                                    let normalized = (lck.pdo_max_voltage - pdo_voltage_v) / voltage_range;
                                    let y_pos = pdo_bar_y + (normalized * (pdo_bar_height - 2) as f32) as i32 + 1;
                                    
                                    // Draw horizontal line extending from left of bar to right edge
                                    Line::new(
                                        Point::new(pdo_bar_x - 3, y_pos),
                                        Point::new(pdo_bar_x + pdo_bar_width as i32, y_pos)
                                    )
                                    .into_styled(PrimitiveStyle::with_stroke(BinaryColor::On, 1))
                                    .draw(&mut display).unwrap();
                                    
                                    // Draw voltage value text next to the line
                                    let voltage_text = if pdo_voltage_v.fract() == 0.0 {
                                        format!("{:.0}", pdo_voltage_v)
                                    } else {
                                        format!("{:.1}", pdo_voltage_v)
                                    };
                                    Text::new(&voltage_text, Point::new(pdo_bar_x - 18, y_pos + 3), style_small)
                                        .draw(&mut display).unwrap();
                                }
                            }
                        }
                        
                        // Draw current set voltage marker (longer horizontal line extending to the left)
                        if lck.output_voltage >= lck.pdo_min_voltage && lck.output_voltage <= lck.pdo_max_voltage {
                            let normalized = (lck.pdo_max_voltage - lck.output_voltage) / voltage_range;
                            let y_pos = pdo_bar_y + (normalized * (pdo_bar_height - 2) as f32) as i32 + 1;
                            
                            // Draw a much longer horizontal line extending left from the bar
                            Line::new(
                                Point::new(pdo_bar_x - 8, y_pos),
                                Point::new(pdo_bar_x + pdo_bar_width as i32, y_pos)
                            )
                            .into_styled(PrimitiveStyle::with_stroke(BinaryColor::On, 1))
                            .draw(&mut display).unwrap();
                        }
                    }
                                                    
                    // Battery status
                    // Text::new(&format!("{:.1}V", battery_voltage), Point::new(86, 60), style_small).draw(&mut display).unwrap();
                                        
                    // Wifi status
                    match lck.wifi {
                        WifiStatus::Disconnected => {
                        },
                        WifiStatus::Connecting => {
                            match loopcount {
                                0..=2 => {
                                    wifi_img0.draw(&mut display).unwrap();
                                },
                                3..=5 => {
                                    wifi_img1.draw(&mut display).unwrap();
                                },
                                6..=8 => {
                                    wifi_img2.draw(&mut display).unwrap();
                                },
                                9..=11 => {
                                    wifi_img3.draw(&mut display).unwrap();
                                },
                                12..=15 => {
                                    wifi_img4.draw(&mut display).unwrap();
                                },
                                _ => {},
                            }
                        },
                        WifiStatus::Connected => {
                            match lck.wifi_rssi {
                                -100..=-80 => {
                                    wifi_img0.draw(&mut display).unwrap();
                                },
                                -79..=-75 => {
                                    wifi_img1.draw(&mut display).unwrap();
                                },
                                -74..=-70 => {
                                    wifi_img2.draw(&mut display).unwrap();
                                },
                                -69..=-65 => {
                                    wifi_img3.draw(&mut display).unwrap();
                                },
                                -64..=-30 => {
                                    wifi_img4.draw(&mut display).unwrap();
                                },
                                _ => {
                                },
                            }
                            if lck.wifi_rssi != 0 {
                                Text::new(&format!("{:+02}dBm", lck.wifi_rssi), Point::new(68, 25), style_small).draw(&mut display).unwrap();
                            }
                            else {
                                Text::new("NO SIG", Point::new(78, 25), style_small).draw(&mut display).unwrap();
                            }
                        },
                    }    
                    
                    // Display Channel
                    // Text::new(&format!("CH:{}", lck.channel), Point::new(50, 50), style_middle).draw(&mut display).unwrap();

                    // Display IP Address (last 2 octets only)
                    let ip_display = if let Some(pos) = lck.ip_address.rfind('.') {
                        if let Some(pos2) = lck.ip_address[..pos].rfind('.') {
                            &lck.ip_address[pos2 + 1..]
                        } else {
                            &lck.ip_address
                        }
                    } else {
                        &lck.ip_address
                    };
                    Text::new(&format!("IP:{}", ip_display), Point::new(50, 50), style_small).draw(&mut display).unwrap();

                    match display.flush() {                  
                        Ok(_) => {},
                        Err(_) => {},
                    }

                    // Update previous values for next comparison
                    prev_voltage = lck.voltage;
                    prev_current = lck.current;
                    prev_power = lck.power;
                    // prev_voltage_range = lck.voltage_range;
                    // prev_current_range = lck.current_range;
                    prev_power_range = lck.power_range;
                    prev_output_voltage = lck.output_voltage;
                    prev_status = match lck.status {
                        LoggingStatus::Start => LoggingStatus::Start,
                        LoggingStatus::Stop => LoggingStatus::Stop,
                    };
                    prev_wifi_status = match lck.wifi {
                        WifiStatus::Disconnected => WifiStatus::Disconnected,
                        WifiStatus::Connecting => WifiStatus::Connecting,
                        WifiStatus::Connected => WifiStatus::Connected,
                    };
                    prev_wifi_rssi = lck.wifi_rssi;
                    prev_buffer_wm = lck.buffer_water_mark;
                    // prev_battery = lck.battery;
                    // prev_battery_level = battery_level;
                    prev_channel = lck.channel;
                    prev_message = lck.message.clone();
                    prev_loopcount_display = loopcount;
                    prev_ip_address = lck.ip_address.clone();
                }
                drop(lck);                
                thread::sleep(Duration::from_millis(100));
            }
        });
    }

    pub fn set_voltage(&mut self, vol: f32, cur: f32, power: f32)
    {
        let mut lck = self.txt.lock().unwrap();
        lck.voltage = vol;
        lck.current = cur;
        lck.power = power;
    }

    pub fn set_current_status(&mut self, status: LoggingStatus)
    {
        let mut lck = self.txt.lock().unwrap();
        lck.status = status;
    }

    pub fn set_wifi_status(&mut self, status: WifiStatus)
    {
        let mut lck = self.txt.lock().unwrap();
        lck.wifi = status;
    }

    pub fn set_message(&mut self, msg: String, enable: bool, timeout: u32)
    {
        let mut lck = self.txt.lock().unwrap();
        lck.message = msg;
        lck.message_enable = enable;
        lck.message_timeout = timeout;
        lck.message_timer = SystemTime::now();
    }

    pub fn set_buffer_watermark(&mut self, wm: u32)
    {
        let mut lck = self.txt.lock().unwrap();
        lck.buffer_water_mark = wm;
    }

    pub fn set_wifi_rssi(&mut self, rssi: i32)
    {
        let mut lck = self.txt.lock().unwrap();
        lck.wifi_rssi = rssi;
    }

    pub fn set_channel(&mut self, channel: u32)
    {
        let mut lck = self.txt.lock().unwrap();
        lck.channel = channel;
    }

    pub fn set_output_voltage(&mut self, output_voltage: f32){
        let mut lck = self.txt.lock().unwrap();
        lck.output_voltage = output_voltage;
    }

    pub fn set_output_voltage_changing(&mut self, changing: bool){
        let mut lck = self.txt.lock().unwrap();
        lck.output_voltage_changing = changing;
    }

    pub fn set_ip_address(&mut self, ip_address: String){
        let mut lck = self.txt.lock().unwrap();
        lck.ip_address = ip_address;
    }

    pub fn set_pdo_range(&mut self, pdo_list: Vec<PDODisplayInfo>, min_voltage: f32, max_voltage: f32) {
        let mut lck = self.txt.lock().unwrap();
        lck.pdo_list = pdo_list;
        lck.pdo_min_voltage = min_voltage;
        lck.pdo_max_voltage = max_voltage;
        info!("PDO range set for display: {:.1}V - {:.1}V, {} PDOs", min_voltage, max_voltage, lck.pdo_list.len());
    }

    /// Display PDO list on startup (static function, doesn't use DisplayPanel state)
    pub fn show_pdo_list(shared_i2c: Arc<Mutex<i2c::I2cDriver<'static>>>, pdo_list: &[crate::usbpd::PDOInfo]) {
        info!("Displaying PDO list on screen...");
        
        // Create I2C wrapper
        struct I2CWrapper {
            driver: Arc<Mutex<i2c::I2cDriver<'static>>>,
        }
        
        impl embedded_hal_0_2::blocking::i2c::Write for I2CWrapper {
            type Error = ();
            
            fn write(&mut self, address: u8, bytes: &[u8]) -> Result<(), Self::Error> {
                let mut driver = self.driver.lock().unwrap();
                driver.write(address, bytes, esp_idf_hal::delay::BLOCK).map_err(|_| ())
            }
        }
        
        let wrapper = I2CWrapper { driver: shared_i2c };
        let interface = I2CDisplayInterface::new(wrapper);
        let mut display = Ssd1306::new(interface, 
            DisplaySize128x64,
            ssd1306::prelude::DisplayRotation::Rotate180)
            .into_buffered_graphics_mode();
        
        if let Err(e) = display.init() {
            info!("Failed to initialize display for PDO list: {:?}", e);
            return;
        }
        
        let text_style = MonoTextStyleBuilder::new()
            .font(&FONT_5X8)
            .text_color(BinaryColor::On)
            .build();
        
        // Calculate number of pages (6 PDOs per page)
        let pdos_per_page = 6;
        let total_pages = (pdo_list.len() + pdos_per_page - 1) / pdos_per_page;
        
        // Display each page for 5 seconds
        for page in 0..total_pages {
            display.clear();
            
            // Title with page number
            let title = if total_pages > 1 {
                format!("PDOs (Page {}/{})", page + 1, total_pages)
            } else {
                "Available PDOs:".to_string()
            };
            
            Text::new(&title, Point::new(0, 7), text_style)
                .draw(&mut display).ok();
            
            // Display PDOs for this page
            let start_idx = page * pdos_per_page;
            let end_idx = ((page + 1) * pdos_per_page).min(pdo_list.len());
            
            for (i, pdo) in pdo_list[start_idx..end_idx].iter().enumerate() {
                let y = 16 + (i as i32 * 8);
                let is_epr = pdo.pdo_index >= 8;
                let type_str = if pdo.is_fixed {
                    if is_epr { "F-EPR" } else { "F-SPR" }
                } else {
                    if is_epr { "AVS" } else { "PPS" }
                };
                
                let line = format!(
                    "{}: {:.1}V {:.1}A {}W {}",
                    pdo.pdo_index,
                    pdo.voltage_mv as f32 / 1000.0,
                    pdo.current_ma as f32 / 1000.0,
                    pdo.max_power_mw / 1000,
                    type_str
                );
                
                Text::new(&line, Point::new(0, y), text_style)
                    .draw(&mut display).ok();
            }
            
            if let Err(e) = display.flush() {
                info!("Failed to flush PDO display: {:?}", e);
            }
            
            info!("PDO page {}/{} displayed", page + 1, total_pages);

            // Wait 3 seconds before showing next page (or finishing)
            thread::sleep(Duration::from_secs(3));
        }
        
        info!("PDO list display complete");
    }
}
