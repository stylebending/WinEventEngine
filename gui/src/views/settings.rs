use iced::widget::{button, checkbox, column, container, pick_list, row, text, Space};
use iced::{Element, Length};

use crate::app::{Message, WinEventApp};
use crate::theme::AppTheme;

pub fn view(app: &WinEventApp) -> Element<Message> {
    let settings_content = column![
        // Theme section
        row![
            text("Theme:"),
            pick_list(
                vec![AppTheme::Dark, AppTheme::Light, AppTheme::System],
                Some(app.theme),
                Message::ThemeChanged,
            ),
        ]
        .spacing(10),
        // Engine controls
        row![
            button("Start Engine").on_press(Message::StartEngine),
            button("Stop Engine").on_press(Message::StopEngine),
        ]
        .spacing(10),
        // Config
        row![button("Reload Config").on_press(Message::ReloadConfig),].spacing(10),
        // Service status
        row![
            text("Service Status:"),
            text(if app.service_installed {
                "Installed"
            } else {
                "Not Installed"
            }),
        ]
        .spacing(10),
        // Service note
        text("Note: Service management requires Administrator privileges")
            .size(12)
            .style(|theme: &iced::Theme| iced::widget::text::Style {
                color: Some(theme.palette().text.scale_alpha(0.7)),
                ..Default::default()
            }),
        // Service management
        row![
            button("Install Service").on_press(Message::InstallService),
            button("Uninstall Service").on_press(Message::UninstallService),
        ]
        .spacing(10),
        // Auto-start
        row![checkbox("Start with Windows", app.service_auto_start)
            .on_toggle(Message::ToggleAutoStart),]
        .spacing(10),
        // Security section
        text("Security").size(16),
        row![
            checkbox("Allow HTTP Request Actions", app.http_requests_enabled)
                .on_toggle(Message::ToggleHttpRequests),
        ]
        .spacing(10),
        text("Enable to allow rules to send HTTP requests to webhooks/APIs")
            .size(12)
            .style(|theme: &iced::Theme| iced::widget::text::Style {
                color: Some(theme.palette().text.scale_alpha(0.6)),
                ..Default::default()
            }),
    ]
    .spacing(20);

    column![
        // Header
        row![text("Settings").size(28),].padding(20),
        // Settings sections
        container(settings_content).padding(20).height(Length::Fill),
    ]
    .into()
}
