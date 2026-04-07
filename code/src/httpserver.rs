// This is mini-power HTTP server module for ESP32-C3-WROOM.
// SPDX-License-Identifier: MIT
// Copyright (c) 2025-2026 Hiroshi Nakajima

use log::*;
use std::sync::{Arc, Mutex};
use esp_idf_svc::io::{Read, Write};
use embedded_svc::http::Headers;
use esp_idf_svc::http::server::{Configuration, EspHttpServer};
use esp_idf_svc::http::Method;

#[derive(Clone, Debug)]
pub struct PDOInfoSimple {
    pub pdo_index: u8,
    pub voltage_mv: u16,
    pub current_ma: u16,
    pub max_power_mw: u32,
    pub is_fixed: bool,
}

/// Shared state for voltage control
#[derive(Clone)]
pub struct PowerControlState {
    pub target_voltage: Arc<Mutex<f32>>,
    pub output_enabled: Arc<Mutex<bool>>,
    pub current_voltage: Arc<Mutex<f32>>,
    pub current_current: Arc<Mutex<f32>>,
    pub current_power: Arc<Mutex<f32>>,
    pub min_voltage: Arc<Mutex<f32>>,
    pub max_voltage: Arc<Mutex<f32>>,
    pub pdo_list: Arc<Mutex<Vec<PDOInfoSimple>>>,
}

impl PowerControlState {
    pub fn new(min_voltage: f32, max_voltage: f32) -> Self {
        Self {
            target_voltage: Arc::new(Mutex::new(0.0)),
            output_enabled: Arc::new(Mutex::new(false)),
            current_voltage: Arc::new(Mutex::new(0.0)),
            current_current: Arc::new(Mutex::new(0.0)),
            current_power: Arc::new(Mutex::new(0.0)),
            min_voltage: Arc::new(Mutex::new(min_voltage)),
            max_voltage: Arc::new(Mutex::new(max_voltage)),
            pdo_list: Arc::new(Mutex::new(Vec::new())),
        }
    }

    pub fn set_pdo_list(&self, pdos: Vec<PDOInfoSimple>) {
        *self.pdo_list.lock().unwrap() = pdos;
        info!("HTTP: PDO list updated with {} entries", self.pdo_list.lock().unwrap().len());
    }

    pub fn get_pdo_list(&self) -> Vec<PDOInfoSimple> {
        self.pdo_list.lock().unwrap().clone()
    }

    pub fn set_target_voltage(&self, voltage: f32) {
        let min_v = *self.min_voltage.lock().unwrap();
        let max_v = *self.max_voltage.lock().unwrap();
        let clamped = voltage.max(min_v).min(max_v);
        info!("HTTP: Setting target voltage {:.3}V -> clamped to {:.3}V (min={:.3}V, max={:.3}V)", 
              voltage, clamped, min_v, max_v);
        *self.target_voltage.lock().unwrap() = clamped;
        info!("HTTP: Target voltage successfully set to {:.3}V", clamped);
    }

    pub fn get_target_voltage(&self) -> f32 {
        *self.target_voltage.lock().unwrap()
    }
    
    pub fn get_min_voltage(&self) -> f32 {
        *self.min_voltage.lock().unwrap()
    }
    
    pub fn get_max_voltage(&self) -> f32 {
        *self.max_voltage.lock().unwrap()
    }

    pub fn set_output_enabled(&self, enabled: bool) {
        *self.output_enabled.lock().unwrap() = enabled;
        info!("HTTP: Output enabled set to {}", enabled);
    }

    pub fn get_output_enabled(&self) -> bool {
        *self.output_enabled.lock().unwrap()
    }

    pub fn update_measurements(&self, voltage: f32, current: f32, power: f32) {
        *self.current_voltage.lock().unwrap() = voltage;
        *self.current_current.lock().unwrap() = current;
        *self.current_power.lock().unwrap() = power;
    }

    pub fn get_measurements(&self) -> (f32, f32, f32) {
        (
            *self.current_voltage.lock().unwrap(),
            *self.current_current.lock().unwrap(),
            *self.current_power.lock().unwrap(),
        )
    }
}

const HTML_PAGE: &str = r#"<!DOCTYPE html>
<html>
<head>
    <meta charset="UTF-8">
    <meta name="viewport" content="width=device-width, initial-scale=1.0">
    <title>Mini Power Control</title>
    <script src="https://cdn.jsdelivr.net/npm/chart.js@4.4.0/dist/chart.umd.min.js"></script>
    <style>
        body {
            font-family: Arial, sans-serif;
            max-width: 800px;
            margin: 50px auto;
            padding: 20px;
            background-color: #f0f0f0;
        }
        .container {
            background-color: white;
            padding: 30px;
            border-radius: 10px;
            box-shadow: 0 2px 10px rgba(0,0,0,0.1);
        }
        h1 {
            color: #333;
            text-align: center;
        }
        .control-group {
            margin: 20px 0;
            padding: 15px;
            background-color: #f9f9f9;
            border-radius: 5px;
        }
        label {
            display: block;
            margin-bottom: 8px;
            font-weight: bold;
            color: #555;
        }
        input[type="number"] {
            width: 100%;
            padding: 10px;
            font-size: 16px;
            border: 1px solid #ddd;
            border-radius: 4px;
            box-sizing: border-box;
        }
        button {
            width: 100%;
            padding: 12px;
            margin: 5px 0;
            font-size: 16px;
            font-weight: bold;
            border: none;
            border-radius: 4px;
            cursor: pointer;
            transition: background-color 0.3s;
        }
        .btn-set {
            background-color: #4CAF50;
            color: white;
        }
        .btn-set:hover {
            background-color: #45a049;
        }
        .btn-on {
            background-color: #2196F3;
            color: white;
        }
        .btn-on:hover {
            background-color: #0b7dda;
        }
        .btn-off {
            background-color: #f44336;
            color: white;
        }
        .btn-off:hover {
            background-color: #da190b;
        }
        .status {
            margin: 20px 0;
            padding: 15px;
            background-color: #e8f5e9;
            border-radius: 5px;
            border-left: 4px solid #4CAF50;
        }
        .status h3 {
            margin-top: 0;
            color: #2e7d32;
        }
        .measurement {
            display: flex;
            justify-content: space-between;
            margin: 8px 0;
            padding: 8px;
            background-color: white;
            border-radius: 3px;
        }
        .measurement-label {
            font-weight: bold;
            color: #555;
        }
        .measurement-value {
            color: #2196F3;
            font-weight: bold;
        }
        #message {
            margin-top: 10px;
            padding: 10px;
            border-radius: 4px;
            display: none;
        }
        .success {
            background-color: #d4edda;
            color: #155724;
            border: 1px solid #c3e6cb;
        }
        .error {
            background-color: #f8d7da;
            color: #721c24;
            border: 1px solid #f5c6cb;
        }
        .chart-container {
            margin: 20px 0;
            padding: 15px;
            background-color: white;
            border-radius: 5px;
            box-shadow: 0 1px 5px rgba(0,0,0,0.1);
        }
        .chart-container h3 {
            margin-top: 0;
            color: #333;
        }
    </style>
</head>
<body>
    <div class="container">
        <h1>⚡ Mini Power Control</h1>
        
        <div class="status">
            <h3>Current Status</h3>
            <div class="measurement">
                <span class="measurement-label">Voltage:</span>
                <span class="measurement-value" id="currentVoltage">--</span>
            </div>
            <div class="measurement">
                <span class="measurement-label">Current:</span>
                <span class="measurement-value" id="currentCurrent">--</span>
            </div>
            <div class="measurement">
                <span class="measurement-label">Power:</span>
                <span class="measurement-value" id="currentPower">--</span>
            </div>
            <div class="measurement">
                <span class="measurement-label">Target:</span>
                <span class="measurement-value" id="targetVoltage">--</span>
            </div>
            <div class="measurement">
                <span class="measurement-label">Output:</span>
                <span class="measurement-value" id="outputStatus">--</span>
            </div>
        </div>

        <div class="status">
            <h3>🔌 Available PDOs</h3>
            <div id="pdoList" style="font-size: 14px;">
                <div style="text-align: center; color: #999;">Loading...</div>
            </div>
        </div>

        <div class="chart-container">
            <h3>📊 Voltage & Current Graph</h3>
            <canvas id="chart"></canvas>
        </div>

        <div class="control-group">
            <label for="voltage">Set Target Voltage (V):</label>
            <div style="display:flex; gap:8px; align-items: center;">
                <button style="width:40px; padding:8px; font-weight:bold;" onclick="decreaseVoltage()">-</button>
                <input type="number" id="voltage" step="0.1" value="0" style="flex:1;">
                <button style="width:40px; padding:8px; font-weight:bold;" onclick="increaseVoltage()">+</button>
            </div>
        </div>

        <div class="control-group">
            <button class="btn-on" onclick="setOutput(true)">Turn ON Output</button>
            <button class="btn-off" onclick="setOutput(false)">Turn OFF Output</button>
        </div>

        <div id="message"></div>
    </div>

    <script>
        function showMessage(text, isError) {
            const msg = document.getElementById('message');
            msg.textContent = text;
            msg.className = isError ? 'error' : 'success';
            msg.style.display = 'block';
            setTimeout(() => {
                msg.style.display = 'none';
            }, 3000);
        }

        function setVoltage() {
            const voltage = document.getElementById('voltage').value;
            fetch('/api/voltage', {
                method: 'POST',
                headers: {'Content-Type': 'application/x-www-form-urlencoded'},
                body: 'value=' + voltage
            })
            .then(response => response.text())
            .then(data => {
                showMessage('Voltage set to ' + voltage + 'V', false);
                updateStatus();
            })
            .catch(error => {
                showMessage('Error setting voltage', true);
            });
        }

        function setOutput(enabled) {
            fetch('/api/output', {
                method: 'POST',
                headers: {'Content-Type': 'application/x-www-form-urlencoded'},
                body: 'value=' + (enabled ? '1' : '0')
            })
            .then(response => response.text())
            .then(data => {
                showMessage('Output ' + (enabled ? 'enabled' : 'disabled'), false);
                updateStatus();
            })
            .catch(error => {
                showMessage('Error setting output', true);
            });
        }

        function increaseVoltage() {
            const input = document.getElementById('voltage');
            const step = parseFloat(input.step) || 0.1;
            const min = parseFloat(input.min) || -Infinity;
            const max = parseFloat(input.max) || Infinity;
            let v = parseFloat(input.value) || 0.0;
            v = Math.min(max, Math.round((v + step) * 10) / 10);
            if (v < min) v = min;
            input.value = v.toFixed(1);
            // Immediately send to server
            setVoltage();
        }

        function decreaseVoltage() {
            const input = document.getElementById('voltage');
            const step = parseFloat(input.step) || 0.1;
            const min = parseFloat(input.min) || -Infinity;
            const max = parseFloat(input.max) || Infinity;
            let v = parseFloat(input.value) || 0.0;
            v = Math.max(min, Math.round((v - step) * 10) / 10);
            if (v > max) v = max;
            input.value = v.toFixed(1);
            // Immediately send to server
            setVoltage();
        }

        function updateStatus() {
            fetch('/api/status')
            .then(response => response.json())
            .then(data => {
                document.getElementById('currentVoltage').textContent = data.voltage.toFixed(3) + ' V';
                document.getElementById('currentCurrent').textContent = data.current.toFixed(3) + ' A';
                document.getElementById('currentPower').textContent = data.power.toFixed(2) + ' W';
                document.getElementById('targetVoltage').textContent = data.target_voltage.toFixed(2) + ' V';
                document.getElementById('outputStatus').textContent = data.output_enabled ? 'ON' : 'OFF';
                
                // Update voltage input min/max based on PDO limits
                const voltageInput = document.getElementById('voltage');
                voltageInput.min = data.min_voltage.toFixed(2);
                voltageInput.max = data.max_voltage.toFixed(2);
                
                // Only update voltage value if the input is not focused (user is not editing)
                if (document.activeElement !== voltageInput) {
                    voltageInput.value = data.target_voltage.toFixed(1);
                }
                
                // Update chart data
                updateChart(data.voltage, data.current);
            })
            .catch(error => {
                console.error('Error fetching status:', error);
            });
        }

        function updatePDOList() {
            fetch('/api/pdos')
            .then(response => response.json())
            .then(data => {
                const pdoListDiv = document.getElementById('pdoList');
                if (data.pdos && data.pdos.length > 0) {
                    let html = '<div style="overflow-x: auto; -webkit-overflow-scrolling: touch;">';
                    html += '<table style="width:100%; border-collapse: collapse; font-size: 13px;">';
                    html += '<tr style="background-color: #f0f0f0; font-weight: bold;">';
                    html += '<th style="padding: 6px; border: 1px solid #ddd; white-space: nowrap;">Index</th>';
                    html += '<th style="padding: 6px; border: 1px solid #ddd; white-space: nowrap;">Voltage</th>';
                    html += '<th style="padding: 6px; border: 1px solid #ddd; white-space: nowrap;">Current</th>';
                    html += '<th style="padding: 6px; border: 1px solid #ddd; white-space: nowrap;">Power</th>';
                    html += '<th style="padding: 6px; border: 1px solid #ddd; white-space: nowrap;">Type</th>';
                    html += '</tr>';
                    data.pdos.forEach(pdo => {
                        const isEPR = pdo.pdo_index >= 8;
                        const typeStr = pdo.is_fixed 
                            ? (isEPR ? 'Fixed EPR' : 'Fixed SPR')
                            : (isEPR ? 'AVS APDO' : 'PPS APDO');
                        html += '<tr>';
                        html += '<td style="padding: 6px; border: 1px solid #ddd; text-align: center;">' + pdo.pdo_index + '</td>';
                        html += '<td style="padding: 6px; border: 1px solid #ddd; text-align: right; white-space: nowrap;">' + (pdo.voltage_mv / 1000).toFixed(2) + ' V</td>';
                        html += '<td style="padding: 6px; border: 1px solid #ddd; text-align: right; white-space: nowrap;">' + (pdo.current_ma / 1000).toFixed(2) + ' A</td>';
                        html += '<td style="padding: 6px; border: 1px solid #ddd; text-align: right; white-space: nowrap;">' + (pdo.max_power_mw / 1000).toFixed(1) + ' W</td>';
                        html += '<td style="padding: 6px; border: 1px solid #ddd; text-align: center; white-space: nowrap;">' + typeStr + '</td>';
                        html += '</tr>';
                    });
                    html += '</table></div>';
                    pdoListDiv.innerHTML = html;
                } else {
                    pdoListDiv.innerHTML = '<div style="text-align: center; color: #999;">No PDOs available</div>';
                }
            })
            .catch(error => {
                console.error('Error fetching PDO list:', error);
                document.getElementById('pdoList').innerHTML = '<div style="text-align: center; color: #f44336;">Error loading PDOs</div>';
            });
        }

        // Chart setup
        const ctx = document.getElementById('chart').getContext('2d');
        const maxDataPoints = 60; // Keep last 60 seconds of data
        const chartData = {
            labels: [],
            datasets: [
                {
                    label: 'Voltage (V)',
                    data: [],
                    borderColor: 'rgb(75, 192, 192)',
                    backgroundColor: 'rgba(75, 192, 192, 0.1)',
                    yAxisID: 'y',
                    tension: 0.3,
                    pointRadius: 0,
                },
                {
                    label: 'Current (A)',
                    data: [],
                    borderColor: 'rgb(255, 99, 132)',
                    backgroundColor: 'rgba(255, 99, 132, 0.1)',
                    yAxisID: 'y1',
                    tension: 0.3,
                    pointRadius: 0,
                }
            ]
        };

        const chart = new Chart(ctx, {
            type: 'line',
            data: chartData,
            options: {
                responsive: true,
                maintainAspectRatio: true,
                aspectRatio: 2,
                interaction: {
                    mode: 'index',
                    intersect: false,
                },
                scales: {
                    x: {
                        display: true,
                        title: {
                            display: true,
                            text: 'Time'
                        },
                        ticks: {
                            maxTicksLimit: 10
                        }
                    },
                    y: {
                        type: 'linear',
                        display: true,
                        position: 'left',
                        title: {
                            display: true,
                            text: 'Voltage (V)',
                            color: 'rgb(75, 192, 192)'
                        },
                        min: 0,
                        grid: {
                            drawOnChartArea: true,
                        },
                    },
                    y1: {
                        type: 'linear',
                        display: true,
                        position: 'right',
                        title: {
                            display: true,
                            text: 'Current (A)',
                            color: 'rgb(255, 99, 132)'
                        },
                        min: 0,
                        grid: {
                            drawOnChartArea: false,
                        },
                    },
                },
                plugins: {
                    legend: {
                        display: true,
                        position: 'top',
                    }
                },
                animation: {
                    duration: 0
                }
            }
        });

        function updateChart(voltage, current) {
            const now = new Date();
            const timeStr = now.toLocaleTimeString('ja-JP', { hour: '2-digit', minute: '2-digit', second: '2-digit' });
            
            chartData.labels.push(timeStr);
            chartData.datasets[0].data.push(voltage);
            chartData.datasets[1].data.push(current);
            
            // Keep only the last maxDataPoints
            if (chartData.labels.length > maxDataPoints) {
                chartData.labels.shift();
                chartData.datasets[0].data.shift();
                chartData.datasets[1].data.shift();
            }
            
            chart.update();
        }

        // Add event listeners for voltage input field
        const voltageInput = document.getElementById('voltage');
        
        // Handle direct input changes (when user types or uses built-in spinner)
        voltageInput.addEventListener('change', function() {
            setVoltage();
        });
        
        // Handle Enter key press
        voltageInput.addEventListener('keypress', function(event) {
            if (event.key === 'Enter') {
                setVoltage();
            }
        });
        
        // Handle input events (fired when value changes via spinner buttons)
        voltageInput.addEventListener('input', function() {
            // Use a timeout to avoid rapid firing
            clearTimeout(this.inputTimeout);
            this.inputTimeout = setTimeout(() => {
                setVoltage();
            }, 500); // 500ms delay
        });

        // Update status every 1 second
        setInterval(updateStatus, 1000);
        // Initial update
        updateStatus();
        // Load PDO list once on page load
        updatePDOList();
    </script>
</body>
</html>
"#;

pub fn start_http_server(state: PowerControlState) -> anyhow::Result<EspHttpServer<'static>> {
    let config = Configuration::default();
    let mut server = EspHttpServer::new(&config)?;

    // Serve main page
    let _state_clone = state.clone();
    server.fn_handler("/", Method::Get, move |request| {
        let mut response = request.into_ok_response()?;
        response.write_all(HTML_PAGE.as_bytes())?;
        Ok::<(), anyhow::Error>(())
    })?;

    // API: Get status
    let state_clone = state.clone();
    server.fn_handler("/api/status", Method::Get, move |request| {
        let target_v = state_clone.get_target_voltage();
        let enabled = state_clone.get_output_enabled();
        let min_v = state_clone.get_min_voltage();
        let max_v = state_clone.get_max_voltage();
        let (voltage, current, power) = state_clone.get_measurements();
        
        let json = format!(
            r#"{{"voltage":{:.3},"current":{:.3},"power":{:.2},"target_voltage":{:.2},"output_enabled":{},"min_voltage":{:.2},"max_voltage":{:.2}}}"#,
            voltage, current, power, target_v, enabled, min_v, max_v
        );
        
        let mut response = request.into_ok_response()?;
        response.write_all(json.as_bytes())?;
        Ok::<(), anyhow::Error>(())
    })?;

    // API: Set voltage
    let state_clone = state.clone();
    server.fn_handler("/api/voltage", Method::Post, move |mut request| {
        let len = request.content_len().unwrap_or(0) as usize;
        let mut buf = vec![0u8; len];
        request.read_exact(&mut buf)?;
        
        let body = String::from_utf8_lossy(&buf);
        if let Some(value_str) = body.strip_prefix("value=") {
            if let Ok(voltage) = value_str.parse::<f32>() {
                state_clone.set_target_voltage(voltage);
                let mut response = request.into_ok_response()?;
                response.write_all(b"OK")?;
                return Ok::<(), anyhow::Error>(());
            } else {
                info!("HTTP API: Failed to parse voltage value: '{}'", value_str);
            }
        } else {
            info!("HTTP API: Request body does not start with 'value=': '{}'", body);
        }
        
        request.into_status_response(400)?.write_all(b"Bad Request")?;
        Ok::<(), anyhow::Error>(())
    })?;

    // API: Set output ON/OFF
    let state_clone = state.clone();
    server.fn_handler("/api/output", Method::Post, move |mut request| {
        let len = request.content_len().unwrap_or(0) as usize;
        let mut buf = vec![0u8; len];
        request.read_exact(&mut buf)?;
        
        let body = String::from_utf8_lossy(&buf);
        if let Some(value_str) = body.strip_prefix("value=") {
            if let Ok(value) = value_str.parse::<u8>() {
                state_clone.set_output_enabled(value != 0);
                let mut response = request.into_ok_response()?;
                response.write_all(b"OK")?;
                return Ok::<(), anyhow::Error>(());
            }
        }
        
        request.into_status_response(400)?.write_all(b"Bad Request")?;
        Ok::<(), anyhow::Error>(())
    })?;

    // API: Get PDO list
    let state_clone = state.clone();
    server.fn_handler("/api/pdos", Method::Get, move |request| {
        let pdos = state_clone.get_pdo_list();
        
        // Build JSON array manually
        let mut json = String::from(r#"{"pdos":["#);
        for (i, pdo) in pdos.iter().enumerate() {
            if i > 0 {
                json.push(',');
            }
            json.push_str(&format!(
                r#"{{"pdo_index":{},"voltage_mv":{},"current_ma":{},"max_power_mw":{},"is_fixed":{}}}"#,
                pdo.pdo_index, pdo.voltage_mv, pdo.current_ma, pdo.max_power_mw, pdo.is_fixed
            ));
        }
        json.push_str("]}");
        
        let mut response = request.into_ok_response()?;
        response.write_all(json.as_bytes())?;
        Ok::<(), anyhow::Error>(())
    })?;

    info!("HTTP server started");
    Ok(server)
}

