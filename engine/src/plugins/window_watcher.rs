use async_trait::async_trait;
use engine_core::event::{Event, EventKind};
use engine_core::plugin::{EventEmitter, EventSourcePlugin, PluginError};
use regex::Regex;
use std::collections::HashMap;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Sender};
use std::thread::{self, JoinHandle};
use std::time::Duration;
use tracing::{error, info, warn};
use windows::Win32::Foundation::HWND;
use windows::Win32::UI::Accessibility::{SetWinEventHook, UnhookWinEvent};
use windows::Win32::UI::WindowsAndMessaging::{DispatchMessageW, GetMessageW, MSG, GetWindowThreadProcessId, PostThreadMessageW};
use windows::Win32::UI::WindowsAndMessaging::{EVENT_SYSTEM_FOREGROUND, EVENT_OBJECT_CREATE, EVENT_OBJECT_DESTROY, EVENT_OBJECT_NAMECHANGE, WINEVENT_OUTOFCONTEXT, WM_QUIT, GetAncestor, GA_ROOT};
use windows::core::PWSTR;
use windows::Win32::System::Threading::{
    OpenProcess, PROCESS_QUERY_INFORMATION, PROCESS_VM_READ, QueryFullProcessImageNameW, PROCESS_NAME_FORMAT, GetCurrentThreadId, GetCurrentProcessId,
};

#[derive(Debug, Clone)]
enum WindowEvent {
    Focused {
        hwnd: HWND,
        title: String,
        process_name: String,
        process_id: u32,
    },
    Created {
        hwnd: HWND,
        title: String,
        process_name: String,
        process_id: u32,
    },
    Destroyed {
        hwnd: HWND,
        title: Option<String>,
    },
    TitleChanged {
        hwnd: HWND,
        title: String,
    },
}

pub struct WindowEventPlugin {
    name: String,
    is_running: Arc<AtomicBool>,
    previous_hwnd: Arc<tokio::sync::Mutex<Option<HWND>>>,
    focused_window_titles: Arc<tokio::sync::Mutex<HashMap<isize, String>>>,
    title_filter: Option<Regex>,
    process_filter: Option<Regex>,
    hook_thread: Option<JoinHandle<()>>,
    hook_thread_id: Option<u32>,
    event_sender: Option<Sender<WindowEvent>>,
}

impl WindowEventPlugin {
    pub fn new(name: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            is_running: Arc::new(AtomicBool::new(false)),
            previous_hwnd: Arc::new(tokio::sync::Mutex::new(None)),
            focused_window_titles: Arc::new(tokio::sync::Mutex::new(HashMap::new())),
            title_filter: None,
            process_filter: None,
            hook_thread: None,
            hook_thread_id: None,
            event_sender: None,
        }
    }

    pub fn with_title_filter(mut self, pattern: impl Into<String>) -> Self {
        let pattern = pattern.into();
        match Regex::new(&pattern) {
            Ok(regex) => self.title_filter = Some(regex),
            Err(e) => warn!("Invalid title filter pattern '{}': {}", pattern, e),
        }
        self
    }

    pub fn with_process_filter(mut self, pattern: impl Into<String>) -> Self {
        let pattern = pattern.into();
        match Regex::new(&pattern) {
            Ok(regex) => self.process_filter = Some(regex),
            Err(e) => warn!("Invalid process filter pattern '{}': {}", pattern, e),
        }
        self
    }

    fn get_window_info(hwnd: HWND) -> Option<(String, u32, String)> {
        if hwnd.0 == 0 {
            return None;
        }

        let mut title_buf = [0u16; 512];
        let len = unsafe { windows::Win32::UI::WindowsAndMessaging::GetWindowTextW(hwnd, &mut title_buf) };
        let title = if len == 0 {
            String::new()
        } else {
            String::from_utf16_lossy(&title_buf[..len as usize])
        };

        let mut process_id = 0u32;
        unsafe { GetWindowThreadProcessId(hwnd, Some(&mut process_id)) };

        let process_name = Self::resolve_process_name(process_id);

        Some((title, process_id, process_name))
    }

    fn resolve_process_name(process_id: u32) -> String {
        if process_id == 0 {
            return String::new();
        }

        unsafe {
            let handle = match OpenProcess(PROCESS_QUERY_INFORMATION | PROCESS_VM_READ, false, process_id) {
                Ok(h) => h,
                Err(_) => return format!("PID:{}", process_id),
            };

            let mut name_buf = [0u16; 260];
            let mut size = 260u32;
            
            if QueryFullProcessImageNameW(handle, PROCESS_NAME_FORMAT(0), PWSTR(name_buf.as_mut_ptr()), &mut size).is_ok() {
                let full_path = String::from_utf16_lossy(&name_buf[..size as usize]);
                if let Some(name) = full_path.rsplit('\\').next() {
                    return name.to_string();
                }
            }
        }
        
        format!("PID:{}", process_id)
    }

    #[allow(dead_code)]
    fn passes_filters(&self, title: &str, process_name: &str) -> bool {
        if let Some(ref title_regex) = self.title_filter {
            if !title_regex.is_match(title) {
                return false;
            }
        }

        if let Some(ref process_regex) = self.process_filter {
            if !process_regex.is_match(process_name) {
                return false;
            }
        }

        true
    }

    fn run_message_loop(
        event_sender: Sender<WindowEvent>,
        is_running: Arc<AtomicBool>,
    ) -> Result<(), String> {
        // Create hooks for different window events
        let foreground_hook = unsafe {
            SetWinEventHook(
                EVENT_SYSTEM_FOREGROUND,
                EVENT_SYSTEM_FOREGROUND,
                None,
                Some(win_event_callback),
                0,
                0,
                WINEVENT_OUTOFCONTEXT,
            )
        };

        let create_hook = unsafe {
            SetWinEventHook(
                EVENT_OBJECT_CREATE,
                EVENT_OBJECT_CREATE,
                None,
                Some(win_event_callback),
                0,
                0,
                WINEVENT_OUTOFCONTEXT,
            )
        };

        let destroy_hook = unsafe {
            SetWinEventHook(
                EVENT_OBJECT_DESTROY,
                EVENT_OBJECT_DESTROY,
                None,
                Some(win_event_callback),
                0,
                0,
                WINEVENT_OUTOFCONTEXT,
            )
        };

        let name_change_hook = unsafe {
            SetWinEventHook(
                EVENT_OBJECT_NAMECHANGE,
                EVENT_OBJECT_NAMECHANGE,
                None,
                Some(win_event_callback),
                0,
                0,
                WINEVENT_OUTOFCONTEXT,
            )
        };

        if foreground_hook.0 == 0 || create_hook.0 == 0 || destroy_hook.0 == 0 || name_change_hook.0 == 0 {
            unsafe {
                if foreground_hook.0 != 0 { let _ = UnhookWinEvent(foreground_hook); }
                if create_hook.0 != 0 { let _ = UnhookWinEvent(create_hook); }
                if destroy_hook.0 != 0 { let _ = UnhookWinEvent(destroy_hook); }
                if name_change_hook.0 != 0 { let _ = UnhookWinEvent(name_change_hook); }
            }
            return Err("Failed to set one or more Windows event hooks".to_string());
        }

        // Store hooks in thread-local storage for cleanup
        let _ = HOOKS.with(|h| {
            *h.borrow_mut() = Some((foreground_hook, create_hook, destroy_hook, name_change_hook));
        });

        // Store sender for callback to use
        let _ = EVENT_SENDER.with(|s| {
            *s.borrow_mut() = Some(event_sender);
        });

        info!("Window event hooks installed, starting message loop");

        let mut msg = MSG::default();
        
        while is_running.load(Ordering::SeqCst) {
            let result = unsafe { GetMessageW(&mut msg, None, 0, 0) };
            
            if result.0 == -1 || result.0 == 0 {
                break;
            }
            
            unsafe { DispatchMessageW(&msg) };
        }

        // Cleanup hooks
        unsafe {
            let _ = UnhookWinEvent(foreground_hook);
            let _ = UnhookWinEvent(create_hook);
            let _ = UnhookWinEvent(destroy_hook);
            let _ = UnhookWinEvent(name_change_hook);
        }

        let _ = HOOKS.with(|h| {
            *h.borrow_mut() = None;
        });

        let _ = EVENT_SENDER.with(|s| {
            *s.borrow_mut() = None;
        });

        Ok(())
    }
}

// Thread-local storage for hooks and sender
thread_local! {
    static HOOKS: std::cell::RefCell<Option<(windows::Win32::UI::Accessibility::HWINEVENTHOOK, windows::Win32::UI::Accessibility::HWINEVENTHOOK, windows::Win32::UI::Accessibility::HWINEVENTHOOK, windows::Win32::UI::Accessibility::HWINEVENTHOOK)>> = std::cell::RefCell::new(None);
    static EVENT_SENDER: std::cell::RefCell<Option<Sender<WindowEvent>>> = std::cell::RefCell::new(None);
}

unsafe extern "system" fn win_event_callback(
    _hook: windows::Win32::UI::Accessibility::HWINEVENTHOOK,
    event: u32,
    hwnd: HWND,
    id_object: i32,
    id_child: i32,
    _id_event_thread: u32,
    _dwms_event_time: u32,
) {
    let hwnd = if hwnd.0 == 0 {
        return;
    } else {
        hwnd
    };

    let event_type = match event {
        EVENT_SYSTEM_FOREGROUND => "focus",
        EVENT_OBJECT_CREATE => "create",
        EVENT_OBJECT_DESTROY => "destroy",
        EVENT_OBJECT_NAMECHANGE => "name_change",
        _ => return,
    };

    EVENT_SENDER.with(|sender| {
        if let Some(ref sender) = *sender.borrow() {
            let window_event = match event_type {
                "focus" => {
                    if let Some((title, process_id, process_name)) = WindowEventPlugin::get_window_info(hwnd) {
                        Some(WindowEvent::Focused {
                            hwnd,
                            title,
                            process_name,
                            process_id,
                        })
                    } else {
                        None
                    }
                }
                "create" => {
                    if let Some((title, process_id, process_name)) = WindowEventPlugin::get_window_info(hwnd) {
                        Some(WindowEvent::Created {
                            hwnd,
                            title,
                            process_name,
                            process_id,
                        })
                    } else {
                        None
                    }
                }
                "destroy" => {
                    let title = WindowEventPlugin::get_window_info(hwnd)
                        .map(|(t, _, _)| t);
                    Some(WindowEvent::Destroyed {
                        hwnd,
                        title,
                    })
                }
                "name_change" => {
                    // Get the new title for name change events
                    let title = WindowEventPlugin::get_window_info(hwnd)
                        .map(|(t, _, _)| t)
                        .unwrap_or_default();
                    info!("Window title changed: hwnd={:?}, title='{}'", hwnd, title);
                    Some(WindowEvent::TitleChanged {
                        hwnd,
                        title,
                    })
                }
                _ => None,
            };

            if let Some(event) = window_event {
                let _ = sender.send(event);
            }
        }
    });
}

#[async_trait]
impl EventSourcePlugin for WindowEventPlugin {
    fn name(&self) -> &str {
        &self.name
    }

    async fn start(&mut self, emitter: EventEmitter) -> Result<(), PluginError> {
        if self.is_running.load(Ordering::SeqCst) {
            return Ok(());
        }

        info!("Starting window event plugin: {}", self.name);

        let plugin_name = self.name.clone();
        let is_running = self.is_running.clone();
        let previous_hwnd = self.previous_hwnd.clone();
        let focused_window_titles = self.focused_window_titles.clone();
        let title_filter = self.title_filter.clone();
        let process_filter = self.process_filter.clone();

        self.is_running.store(true, Ordering::SeqCst);

        // Get current process ID to filter out GUI windows
        let current_process_id = unsafe { GetCurrentProcessId() };

        // Create channel for thread communication
        let (event_sender, event_receiver) = mpsc::channel::<WindowEvent>();
        self.event_sender = Some(event_sender.clone());

        // Create channel to receive thread ID from spawned thread
        let (thread_id_tx, thread_id_rx) = mpsc::channel::<u32>();

        // Spawn dedicated thread for Windows message loop
        let is_running_clone = is_running.clone();
        let hook_thread = thread::spawn(move || {
            // Send thread ID back to main thread
            let thread_id = unsafe { GetCurrentThreadId() };
            let _ = thread_id_tx.send(thread_id);
            
            if let Err(e) = Self::run_message_loop(event_sender, is_running_clone) {
                error!("Window event hook thread failed: {}", e);
            }
        });
        
        // Receive thread ID (with timeout to avoid blocking if thread fails immediately)
        self.hook_thread_id = thread_id_rx.recv_timeout(Duration::from_secs(1)).ok();
        self.hook_thread = Some(hook_thread);

        // Give the hook a moment to register
        tokio::time::sleep(Duration::from_millis(50)).await;

        // Check if thread is still running (hooks were set successfully)
        if self.hook_thread.as_ref().map(|t| t.is_finished()).unwrap_or(true) {
            self.is_running.store(false, Ordering::SeqCst);
            self.hook_thread = None;
            self.event_sender = None;
            return Err(PluginError::Initialization(
                "Failed to install Windows event hooks".to_string()
            ));
        }

        // Spawn async task to process events from the thread
        tokio::spawn(async move {
            info!("Window event monitoring active (real-time via SetWinEventHook)");

            while is_running.load(Ordering::SeqCst) {
                match event_receiver.try_recv() {
                    Ok(window_event) => {
                        match window_event {
                            WindowEvent::Focused { hwnd, title, process_name, process_id } => {
                                // Check if this window belongs to our own process (GUI window)
                                // We track it for proper focus chain management, but don't emit events
                                let is_own_process = process_id == current_process_id;

                                // Check filters
                                let passes = if let Some(ref title_regex) = title_filter {
                                    title_regex.is_match(&title)
                                } else {
                                    true
                                } && if let Some(ref process_regex) = process_filter {
                                    process_regex.is_match(&process_name)
                                } else {
                                    true
                                };

                                if !passes {
                                    continue;
                                }

                                // Send unfocus event for previous window
                                let mut prev_guard = previous_hwnd.lock().await;
                                if let Some(prev_hwnd) = *prev_guard {
                                    if prev_hwnd.0 != hwnd.0 {
                                        // Only emit unfocus event if previous window is not our own
                                        let prev_process_id = {
                                            let mut pid = 0u32;
                                            unsafe { GetWindowThreadProcessId(prev_hwnd, Some(&mut pid)) };
                                            pid
                                        };
                                        if prev_process_id != current_process_id {
                                            if let Some((prev_title, _, _)) = WindowEventPlugin::get_window_info(prev_hwnd) {
                                                let unfocus_event = Event::new(
                                                    EventKind::WindowUnfocused {
                                                        hwnd: prev_hwnd.0 as isize,
                                                        title: prev_title.clone(),
                                                    },
                                                    &plugin_name,
                                                ).with_metadata("window_title", &prev_title);

                                                let _ = emitter.try_send(unfocus_event);
                                            }
                                        }
                                    }
                                }

                                // Check if this is the same window but title changed (browser tab switch scenario)
                                let hwnd_key = hwnd.0 as isize;
                                let mut titles_guard = focused_window_titles.lock().await;
                                
                                if let Some(old_title) = titles_guard.get(&hwnd_key) {
                                    // Same window already tracked - check if title changed
                                    if old_title != &title {
                                        info!("Same window, title changed: '{}' -> '{}'", old_title, title);
                                        
                                        // Check if title pattern match status changed
                                        let old_matches = if let Some(ref title_regex) = title_filter {
                                            title_regex.is_match(old_title)
                                        } else {
                                            true
                                        };
                                        
                                        let new_matches = if let Some(ref title_regex) = title_filter {
                                            title_regex.is_match(&title)
                                        } else {
                                            true
                                        };
                                        
                                        info!("Pattern matching: old_matches={}, new_matches={}", old_matches, new_matches);
                                        
                                        // Pattern match status changed - emit synthetic focus/unfocus events
                                        if !old_matches && new_matches {
                                            // Started matching - emit WindowFocused
                                            info!("Title changed to match pattern: '{}' -> '{}'", old_title, title);
                                            let focus_event = Event::new(
                                                EventKind::WindowFocused {
                                                    hwnd: hwnd_key,
                                                    title: title.clone(),
                                                },
                                                &plugin_name,
                                            )
                                            .with_metadata("window_title", &title)
                                            .with_metadata("title_changed", "true");
                                            
                                            let _ = emitter.try_send(focus_event);
                                        } else if old_matches && !new_matches {
                                            // Stopped matching - emit WindowUnfocused
                                            info!("Title changed to no longer match pattern: '{}' -> '{}'", old_title, title);
                                            let unfocus_event = Event::new(
                                                EventKind::WindowUnfocused {
                                                    hwnd: hwnd_key,
                                                    title: old_title.clone(),
                                                },
                                                &plugin_name,
                                            )
                                            .with_metadata("window_title", old_title)
                                            .with_metadata("title_changed", "true");
                                            
                                            let _ = emitter.try_send(unfocus_event);
                                        }
                                    }
                                }
                                
                                // Update stored title
                                titles_guard.insert(hwnd_key, title.clone());
                                drop(titles_guard);
                                
                                // Only emit focus event if this is not our own process window
                                if !is_own_process {
                                    let focus_event = Event::new(
                                        EventKind::WindowFocused {
                                            hwnd: hwnd.0 as isize,
                                            title: title.clone(),
                                        },
                                        &plugin_name,
                                    )
                                    .with_metadata("window_title", &title)
                                    .with_metadata("process_id", process_id.to_string())
                                    .with_metadata("process_name", &process_name);

                                    let _ = emitter.try_send(focus_event);
                                }
                                
                                *prev_guard = Some(hwnd);
                            }
                            WindowEvent::Created { hwnd, title, process_name, process_id } => {
                                // Skip windows from our own process (GUI windows)
                                if process_id == current_process_id {
                                    continue;
                                }

                                // Check filters
                                let passes = if let Some(ref title_regex) = title_filter {
                                    title_regex.is_match(&title)
                                } else {
                                    true
                                } && if let Some(ref process_regex) = process_filter {
                                    process_regex.is_match(&process_name)
                                } else {
                                    true
                                };

                                if !passes {
                                    continue;
                                }

                                let create_event = Event::new(
                                    EventKind::WindowCreated {
                                        hwnd: hwnd.0 as isize,
                                        title: title.clone(),
                                        process_id,
                                    },
                                    &plugin_name,
                                )
                                .with_metadata("window_title", &title)
                                .with_metadata("process_name", &process_name);

                                let _ = emitter.try_send(create_event);
                            }
                            WindowEvent::Destroyed { hwnd, title } => {
                                // Check if destroyed window belongs to our own process
                                let process_id = {
                                    let mut pid = 0u32;
                                    unsafe { GetWindowThreadProcessId(hwnd, Some(&mut pid)) };
                                    pid
                                };
                                if process_id == current_process_id {
                                    continue;
                                }

                                let destroyed_event = Event::new(
                                    EventKind::WindowDestroyed {
                                        hwnd: hwnd.0 as isize,
                                    },
                                    &plugin_name,
                                );

                                let destroyed_event = if let Some(ref t) = title {
                                    destroyed_event.with_metadata("window_title", t)
                                } else {
                                    destroyed_event
                                };

                                let _ = emitter.try_send(destroyed_event);
                                
                                // Remove from title tracking
                                {
                                    let mut titles_guard = focused_window_titles.lock().await;
                                    titles_guard.remove(&(hwnd.0 as isize));
                                }
                            }
                            WindowEvent::TitleChanged { hwnd, title } => {
                                // Get the root window - browsers fire events on child objects
                                let root_hwnd = unsafe { GetAncestor(hwnd, GA_ROOT) };
                                let root_hwnd_key = root_hwnd.0 as isize;
                                let event_hwnd_key = hwnd.0 as isize;
                                
                                // Check process of the ROOT window
                                let process_id = {
                                    let mut pid = 0u32;
                                    unsafe { GetWindowThreadProcessId(root_hwnd, Some(&mut pid)) };
                                    pid
                                };
                                
                                // Skip our own process windows
                                if process_id == current_process_id {
                                    continue;
                                }
                                
                                // Check if the ROOT window is in our focused windows tracking
                                let mut titles_guard = focused_window_titles.lock().await;
                                
                                // Try to find the window - either the root or check if event hwnd matches directly
                                let (lookup_hwnd_key, old_title_opt) = if let Some(old) = titles_guard.get(&root_hwnd_key) {
                                    (root_hwnd_key, Some(old.clone()))
                                } else if let Some(old) = titles_guard.get(&event_hwnd_key) {
                                    (event_hwnd_key, Some(old.clone()))
                                } else {
                                    // Window not in tracking - skip
                                    continue;
                                };
                                
                                if let Some(old_title) = old_title_opt {
                                    // Only process if title actually changed
                                    if old_title != title {
                                        // Always emit synthetic unfocus for old title and focus for new title
                                        // This allows the rule engine to properly check title_contains patterns
                                        
                                        // Emit WindowUnfocused for the old title
                                        let unfocus_event = Event::new(
                                            EventKind::WindowUnfocused {
                                                hwnd: lookup_hwnd_key,
                                                title: old_title.clone(),
                                            },
                                            &plugin_name,
                                        )
                                        .with_metadata("window_title", old_title)
                                        .with_metadata("title_changed", "true");
                                        
                                        let _ = emitter.try_send(unfocus_event);
                                        
                                        // Emit WindowFocused for the new title
                                        let focus_event = Event::new(
                                            EventKind::WindowFocused {
                                                hwnd: lookup_hwnd_key,
                                                title: title.clone(),
                                            },
                                            &plugin_name,
                                        )
                                        .with_metadata("window_title", &title)
                                        .with_metadata("title_changed", "true");
                                        
                                        let _ = emitter.try_send(focus_event);
                                        
                                        // Update stored title
                                        titles_guard.insert(lookup_hwnd_key, title);
                                    }
                                }
                            }
                        }
                    }
                    Err(mpsc::TryRecvError::Empty) => {
                        tokio::time::sleep(Duration::from_millis(1)).await;
                    }
                    Err(mpsc::TryRecvError::Disconnected) => {
                        break;
                    }
                }
            }

            info!("Window event processing stopped");
        });

        info!("Window event monitoring active");
        Ok(())
    }

    async fn stop(&mut self) -> Result<(), PluginError> {
        info!("Stopping window event plugin: {}", self.name);
        self.is_running.store(false, Ordering::SeqCst);
        
        // Signal sender to drop (which will cause thread to exit)
        self.event_sender = None;
        
        // Post WM_QUIT to wake up the message loop thread
        if let Some(thread_id) = self.hook_thread_id {
            unsafe {
                let _ = PostThreadMessageW(thread_id, WM_QUIT, None, None);
            }
        }
        
        // Wait for thread to finish (with timeout to avoid hanging)
        if let Some(thread) = self.hook_thread.take() {
            let _ = thread.join();
        }
        
        self.hook_thread_id = None;
        
        Ok(())
    }

    fn is_running(&self) -> bool {
        self.is_running.load(Ordering::SeqCst)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use engine_core::plugin::EventSourcePlugin;

    #[tokio::test]
    async fn test_window_plugin_lifecycle() {
        let (tx, _rx) = tokio::sync::mpsc::channel(10);
        let mut plugin = WindowEventPlugin::new("test_window");

        assert!(!plugin.is_running());

        plugin.start(tx).await.expect("Failed to start plugin");
        assert!(plugin.is_running());

        plugin.stop().await.expect("Failed to stop plugin");
        assert!(!plugin.is_running());
    }

    #[test]
    fn test_title_filter() {
        let plugin = WindowEventPlugin::new("test")
            .with_title_filter("notepad.*");
        
        assert!(plugin.passes_filters("notepad.exe", "notepad.exe"));
        assert!(plugin.passes_filters("notepad++", "notepad++.exe"));
        assert!(!plugin.passes_filters("chrome.exe", "chrome.exe"));
    }

    #[test]
    fn test_process_filter() {
        let plugin = WindowEventPlugin::new("test")
            .with_process_filter("chrome.*");
        
        assert!(plugin.passes_filters("Google Chrome", "chrome.exe"));
        assert!(!plugin.passes_filters("Notepad", "notepad.exe"));
    }

    #[test]
    fn test_combined_filters() {
        let plugin = WindowEventPlugin::new("test")
            .with_title_filter(".*Chrome.*")
            .with_process_filter("chrome.*");
        
        assert!(plugin.passes_filters("Google Chrome", "chrome.exe"));
        assert!(!plugin.passes_filters("Notepad", "notepad.exe"));
        assert!(!plugin.passes_filters("Google Chrome", "firefox.exe"));
    }
}
