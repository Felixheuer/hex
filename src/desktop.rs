pub(crate) mod activity;
#[cfg_attr(target_os = "linux", allow(dead_code))]
pub(crate) mod automation;
pub(crate) mod background_process;
#[cfg_attr(not(target_os = "macos"), allow(dead_code))]
pub(crate) mod copies;
#[cfg_attr(any(target_os = "linux", target_os = "windows"), allow(dead_code))]
pub(crate) mod events;
pub(crate) mod file_dialog;
pub(crate) mod host;
pub(crate) mod i18n;
pub(crate) mod instance;
pub(crate) mod keep_awake;
#[cfg_attr(target_os = "linux", allow(dead_code))]
pub(crate) mod login_item;
#[cfg_attr(any(target_os = "linux", target_os = "windows"), allow(dead_code))]
pub(crate) mod onboarding;
pub(crate) mod opencode_catalog;
pub(crate) mod opencode_service;
#[cfg_attr(not(target_os = "macos"), allow(dead_code))]
pub(crate) mod permission_doctor;
#[cfg(target_os = "macos")]
pub(crate) mod permission_guide;
#[cfg_attr(not(target_os = "macos"), allow(dead_code))]
pub(crate) mod process_exit;
pub(crate) mod screen_lock;
pub(crate) mod setup_gaps;
#[cfg(any(target_os = "linux", target_os = "windows"))]
pub(crate) mod tray_icon;
pub(crate) mod tray_menu;
