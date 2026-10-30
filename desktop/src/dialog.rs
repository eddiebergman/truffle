//! Native confirmation alerts.
pub struct Confirmation<'a> {
    pub title: &'a str,
    pub message: &'a str,
    /// Label of the default button. The other button is Cancel.
    pub accept: &'a str,
}
/// Shows a system alert and blocks until the user answers. Never call it on the main thread.
#[cfg(target_os = "macos")]
pub fn confirm(confirmation: &Confirmation) -> bool {
    use objc2_core_foundation::{
        kCFUserNotificationDefaultResponse, kCFUserNotificationNoteAlertLevel, CFString,
        CFUserNotification,
    };
    let title = CFString::from_str(confirmation.title);
    let message = CFString::from_str(confirmation.message);
    let accept = CFString::from_str(confirmation.accept);
    let cancel = CFString::from_str("Cancel");
    let mut response = 0;
    let failed = unsafe {
        CFUserNotification::display_alert(
            0.0,
            kCFUserNotificationNoteAlertLevel,
            None,
            None,
            None,
            Some(&title),
            Some(&message),
            Some(&accept),
            Some(&cancel),
            None,
            &raw mut response,
        )
    };
    failed == 0 && response == kCFUserNotificationDefaultResponse
}
#[cfg(not(target_os = "macos"))]
pub fn confirm(_: &Confirmation) -> bool {
    false
}
