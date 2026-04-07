// This is mini-power key event module for ESP32-C3-WROOM.
// SPDX-License-Identifier: MIT
// Copyright (c) 2025-2026 Hiroshi Nakajima

use log::*;
use std::{thread, time::Duration, sync::Arc, sync::Mutex, sync::atomic::AtomicBool};
use esp_idf_hal::{gpio::*};
use std::sync::atomic::Ordering;
use std::time::SystemTime;

type PindriverUp = Box<PinDriver<'static, esp_idf_hal::gpio::Gpio10, esp_idf_hal::gpio::Input>>; // GPIO10
type PindriverDown = Box<PinDriver<'static, esp_idf_hal::gpio::Gpio20, esp_idf_hal::gpio::Input>>; // GPIO20
type PindriverCenter = Box<PinDriver<'static, esp_idf_hal::gpio::Gpio21, esp_idf_hal::gpio::Input>>; // GPIO21

static GPIO_UP_FLAG: AtomicBool = AtomicBool::new(false);
static GPIO_DOWN_FLAG: AtomicBool = AtomicBool::new(false);
static GPIO_CENTER_FLAG: AtomicBool = AtomicBool::new(false);

const GARD_TIME: u128 = 100;    // if the time difference is less than 30ms, ignore the interrupt
const KEY_SLEEP_TIME: u64 = 10; // scan the key every 10ms

#[allow(dead_code)]
pub enum Key {
    Up,
    Down,
    Left,
    Right,
    Center,
}

#[allow(dead_code)]
#[derive(Debug, Clone, Copy)]
pub enum KeyEvent {
    UpKeyDown,
    UpKeyUp,
    DownKeyDown,
    DownKeyUp,
    LeftKeyDown,
    LeftKeyUp,
    RightKeyDown,
    RightKeyUp,
    CenterKeyDown,
    CenterKeyUp,
    UpDownLongPress,  // Up and Down pressed simultaneously for >1 second
}

struct KeyState {
    up: bool,
    up_count: u32,
    down: bool,
    down_count: u32,
    left: bool,
    left_count: u32,
    right: bool,
    right_count: u32,
    center: bool,
    center_count: u32,
    key_envet: Vec<KeyEvent>,
    key_sleep: bool,
    up_down_both_pressed_start: Option<SystemTime>,  // Track when both Up and Down are pressed
    up_down_long_press_triggered: bool,  // Prevent repeated triggers
}

pub struct KeySwitch {
    state: Arc<Mutex<KeyState>>
}

#[allow(dead_code)]
impl KeySwitch {
    pub fn new() -> KeySwitch {
        KeySwitch { state: Arc::new(Mutex::new(
            KeyState { 
                up :    false,  up_count:       0,
                down:   false,  down_count:     0,
                left:   false,  left_count:     0,
                right:  false,  right_count:    0,
                center: false,  center_count:   0,
                key_envet: Vec::new(),
                key_sleep: false,
                up_down_both_pressed_start: None,
                up_down_long_press_triggered: false,
            }))}
    }

    // UP: GPIO4
    pub fn gpio_up_interrupt_handler() {
        GPIO_UP_FLAG.store(true, Ordering::Relaxed);
    }

    // DOWN: GPIO5
    pub fn gpio_down_interrupt_handler() {
        GPIO_DOWN_FLAG.store(true, Ordering::Relaxed);
    }

    // CENTER: GPIO21
    pub fn gpio_center_interrupt_handler() {
        GPIO_CENTER_FLAG.store(true, Ordering::Relaxed);
    }

    pub fn start(&mut self,
            mut gpio_up_sig : PindriverUp,
            mut gpio_down_sig : PindriverDown,
            mut gpio_center_sig : PindriverCenter
        )
    {
        let state = self.state.clone();
        let _th = thread::spawn(move || {
            info!("Start Switch Read Thread.");
            gpio_up_sig.set_pull(Pull::Up).unwrap();
            gpio_down_sig.set_pull(Pull::Up).unwrap();
            gpio_center_sig.set_pull(Pull::Up).unwrap();
 
            gpio_up_sig.set_interrupt_type(InterruptType::AnyEdge).unwrap();
            gpio_down_sig.set_interrupt_type(InterruptType::AnyEdge).unwrap();
            gpio_center_sig.set_interrupt_type(InterruptType::AnyEdge).unwrap();
 
            unsafe {
                gpio_up_sig.subscribe(KeySwitch::gpio_up_interrupt_handler).unwrap();
                gpio_down_sig.subscribe(KeySwitch::gpio_down_interrupt_handler).unwrap();
                gpio_center_sig.subscribe(KeySwitch::gpio_center_interrupt_handler).unwrap();
            }

            gpio_up_sig.enable_interrupt().unwrap();
            gpio_down_sig.enable_interrupt().unwrap();
            gpio_center_sig.enable_interrupt().unwrap();
 
            // caluculate the time difference between the last interrupt and now
            // if the time differrence is less than 1sec, igonore the interrupt
            let mut last_interrupt_time_up = SystemTime::now();
            let mut last_interrupt_time_down = SystemTime::now();
            let mut last_interrupt_time_center = SystemTime::now();
            // let mut last_interrupt_time_right = SystemTime::now();
            // let mut last_interrupt_time_center = SystemTime::now();
            // let mut last_interrupt_time_left = SystemTime::now();
            loop {
                let mut lck = state.lock().unwrap();
                if lck.key_sleep == true {
                    lck.key_envet.clear();
                    drop(lck);
                    thread::sleep(Duration::from_millis(KEY_SLEEP_TIME));
                    continue;
                }

                // UP: GPIO_UP
                if GPIO_UP_FLAG.load(Ordering::Relaxed) {
                    match last_interrupt_time_up.elapsed(){
                        Ok(elapsed) => {
                            if elapsed.as_millis() > GARD_TIME {
                                if lck.up == false {
                                    lck.up = true;
                                    lck.up_count = 0;
                                    lck.key_envet.push(KeyEvent::UpKeyDown);
                                    // info!("PUSH_NOTIFICATION_UP_KEY_DOWN");
                                }
                                else {
                                    lck.up = false;
                                    lck.up_count = match last_interrupt_time_up.elapsed() {
                                        Ok(elapsed) => elapsed.as_millis() as u32,
                                        Err(e) => {
                                            error!("Error: {:?}", e);
                                            0
                                        }
                                    };
                                    lck.key_envet.push(KeyEvent::UpKeyUp);
                                    // info!("PUSH_NOTIFICATION_UP_KEY_UP {}", lck.up_count);
                                }
                                last_interrupt_time_up = SystemTime::now();
                            }
                        },
                        Err(e) => {
                            error!("Error: {:?}", e);
                        }
                    }
                    GPIO_UP_FLAG.store(false, Ordering::Relaxed);
                    gpio_up_sig.enable_interrupt().unwrap();
                }
                // DOWN: GPIO_DOWN
                if GPIO_DOWN_FLAG.load(Ordering::Relaxed) {
                    match last_interrupt_time_down.elapsed() {
                        Ok(elapsed) => {
                            if elapsed.as_millis() > GARD_TIME {
                                if lck.down == false {
                                    lck.down = true;
                                    lck.down_count = 0;
                                    lck.key_envet.push(KeyEvent::DownKeyDown);
                                    // info!("PUSH_NOTIFICATION_DOWN_KEY_DOWN");
                                }
                                else {
                                    lck.down = false;
                                    lck.down_count = match last_interrupt_time_down.elapsed() {
                                        Ok(elapsed) => elapsed.as_millis() as u32,
                                        Err(e) => {
                                            error!("Error: {:?}", e);
                                            0
                                        }
                                    };
                                    lck.key_envet.push(KeyEvent::DownKeyUp);
                                    // info!("PUSH_NOTIFICATION_DOWN_KEY_UP {}", lck.down_count);
                                }
                                last_interrupt_time_down = SystemTime::now();
                            }
                        },
                        Err(e) => {
                            error!("Error: {:?}", e);
                        }
                    }
                    GPIO_DOWN_FLAG.store(false, Ordering::Relaxed);
                    gpio_down_sig.enable_interrupt().unwrap();
                }
                // CENTER: GPIO_CENTER
                if GPIO_CENTER_FLAG.load(Ordering::Relaxed) {
                    match last_interrupt_time_center.elapsed() {
                        Ok(elapsed) => {
                            if elapsed.as_millis() > GARD_TIME {
                                if lck.center == false {
                                    lck.center = true;
                                    lck.center_count = 0;
                                    lck.key_envet.push(KeyEvent::CenterKeyDown);
                                    // info!("PUSH_NOTIFICATION_CENTER_KEY_DOWN");
                                }
                                else {
                                    lck.center = false;
                                    lck.center_count = match last_interrupt_time_center.elapsed() {
                                        Ok(elapsed) => elapsed.as_millis() as u32,
                                        Err(e) => {
                                            error!("Error: {:?}", e);
                                            0
                                        }
                                    };
                                    lck.key_envet.push(KeyEvent::CenterKeyUp);
                                    // info!("PUSH_NOTIFICATION_CENTER_KEY_UP {}", lck.center_count);
                                }
                                last_interrupt_time_center = SystemTime::now();
                            }
                        },
                        Err(e) => {
                            error!("Error: {:?}", e);
                        }
                    }
                    GPIO_CENTER_FLAG.store(false, Ordering::Relaxed);
                    gpio_center_sig.enable_interrupt().unwrap();
                }
                
                // Check for simultaneous Up+Down long press (>1 second)
                // Read physical GPIO state directly instead of relying on interrupt timing
                let up_pressed = gpio_up_sig.is_low();
                let down_pressed = gpio_down_sig.is_low();
                
                if up_pressed && down_pressed {
                    // Both keys are physically pressed
                    if lck.up_down_both_pressed_start.is_none() {
                        // Start tracking the press time
                        lck.up_down_both_pressed_start = Some(SystemTime::now());
                        lck.up_down_long_press_triggered = false;
                    } else if !lck.up_down_long_press_triggered {
                        // Check if 1 second has elapsed
                        if let Some(start_time) = lck.up_down_both_pressed_start {
                            if let Ok(elapsed) = start_time.elapsed() {
                                if elapsed.as_millis() >= 1000 {
                                    // Trigger the event
                                    lck.key_envet.push(KeyEvent::UpDownLongPress);
                                    lck.up_down_long_press_triggered = true;
                                    info!("Up+Down long press detected!");
                                }
                            }
                        }
                    }
                } else {
                    // At least one key is released, reset the tracking
                    lck.up_down_both_pressed_start = None;
                    lck.up_down_long_press_triggered = false;
                }
                
                drop(lck);
                thread::sleep(Duration::from_millis(KEY_SLEEP_TIME));
            }
        });
    }

    pub fn get_current_button_state(&mut self, button: Key) -> bool
    {
        let lock= self.state.lock().unwrap();
        match button {
            Key::Up => {
                let ret = lock.up;
                ret
            },
            Key::Down => {
                let ret = lock.down;
                ret
            },
            Key::Left => {
                let ret = lock.left;
                ret
            },
            Key::Right => {
                let ret = lock.right;
                ret
            },
            Key::Center => {
                let ret = lock.center;
                ret
            },            
        }
    }

    pub fn get_button_press_time(&mut self, button: Key) -> u32
    {
        let lock= self.state.lock().unwrap();
        match button {
            Key::Up => {
                let ret = lock.up_count;
                ret
            },
            Key::Down => {
                let ret = lock.down_count;
                ret
            },
            Key::Left => {
                let ret = lock.left_count;
                ret
            },
            Key::Right => {
                let ret = lock.right_count;
                ret
            },
            Key::Center => {
                let ret = lock.center_count;
                ret
            },
        }
    }

    pub fn clear_all_button_event(&mut self)
    {
        let mut lock= self.state.lock().unwrap();
        lock.key_envet.clear();
    }

    pub fn get_key_event_and_clear(&mut self) -> Vec<KeyEvent>
    {
        let mut lock= self.state.lock().unwrap();
        let ret = lock.key_envet.clone();
        lock.key_envet.clear();
        ret
    }

    pub fn set_key_sleep(&mut self, sleep: bool)
    {
        let mut lock= self.state.lock().unwrap();
        lock.key_sleep = sleep;
    }
}
