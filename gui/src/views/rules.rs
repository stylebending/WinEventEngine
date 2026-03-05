use iced::widget::{
    button, checkbox, column, container, pick_list, row, scrollable, text, text_input, Space,
};
use iced::{Alignment, Element, Length};

use crate::app::{Message, WinEventApp};

pub fn view(app: &WinEventApp) -> Element<Message> {
    let rules_list: Element<Message> = if app.rules.is_empty() {
        column![
            text("No automations configured yet."),
            text("Click 'New Automation' to create one."),
        ]
        .spacing(10)
        .into()
    } else {
        scrollable(
            column(
                app.rules
                    .iter()
                    .map(|rule| {
                        row![
                            checkbox("", rule.enabled).on_toggle(|enabled| Message::ToggleRule(
                                rule.name.clone(),
                                enabled
                            )),
                            text(&rule.name).width(200),
                            text(format!("{:?}", rule.trigger)).width(200),
                            text(format!("{:?}", rule.action)).width(200),
                            Space::with_width(Length::Fill),
                            button("Edit").on_press(Message::EditRule(rule.name.clone())),
                            button("Delete").on_press(Message::DeleteRule(rule.name.clone())),
                        ]
                        .spacing(10)
                        .align_y(Alignment::Center)
                        .into()
                    })
                    .collect::<Vec<_>>(),
            )
            .spacing(10),
        )
        .into()
    };

    column![
        row![
            text("Automations").size(28),
            Space::with_width(Length::Fill),
            button("+ New Automation").on_press(Message::CreateRule),
        ]
        .spacing(20)
        .padding(20),
        container(rules_list).padding(20).height(Length::Fill),
        row![
            button("Import").on_press(Message::ImportRules),
            button("Export").on_press(Message::ExportRules),
        ]
        .spacing(10)
        .padding(20),
    ]
    .into()
}

pub fn view_editor(app: &WinEventApp) -> Element<Message> {
    let title = if let Some(name) = &app.editing_rule {
        format!("Edit Automation: {}", name)
    } else {
        "New Automation".to_string()
    };

    // Basic Info Section
    let basic_info = column![
        text("Basic Information").size(18),
        text_input("Rule Name", &app.rule_name)
            .on_input(Message::RuleNameChanged)
            .padding(10),
        text_input("Description (optional)", &app.rule_description)
            .on_input(Message::RuleDescriptionChanged)
            .padding(10),
        checkbox("Enabled", app.rule_enabled).on_toggle(Message::RuleEnabledChanged),
    ]
    .spacing(10);

    // Trigger Section
    let trigger_section = column![
        text("Trigger").size(18),
        pick_list(
            vec![
                "window_focused".to_string(),
                "window_unfocused".to_string(),
                "window_created".to_string(),
                "process_started".to_string(),
                "process_stopped".to_string(),
                "file_created".to_string(),
                "file_modified".to_string(),
                "file_deleted".to_string(),
                "timer".to_string(),
            ],
            Some(app.trigger_type.clone()),
            Message::TriggerTypeChanged,
        ),
    ];

    // Trigger-specific fields
    let trigger_fields: Element<Message> = match app.trigger_type.as_str() {
        "file_created" | "file_modified" | "file_deleted" => column![
            text_input("Path (optional)", &app.trigger_path)
                .on_input(Message::TriggerPathChanged)
                .padding(10),
            text_input("Pattern (e.g., *.txt)", &app.trigger_pattern)
                .on_input(Message::TriggerPatternChanged)
                .padding(10),
        ]
        .spacing(5)
        .into(),
        "window_focused" | "window_unfocused" => column![
            text_input("Title Contains (optional)", &app.trigger_title_contains)
                .on_input(Message::TriggerTitleContainsChanged)
                .padding(10),
            text_input("Process Name (optional)", &app.trigger_process_name)
                .on_input(Message::TriggerProcessNameChanged)
                .padding(10),
        ]
        .spacing(5)
        .into(),
        "process_started" | "process_stopped" => {
            column![
                text_input("Process Name (optional)", &app.trigger_process_name)
                    .on_input(Message::TriggerProcessNameChanged)
                    .padding(10),
            ]
            .into()
        }
        "timer" => column![text_input("Interval (seconds)", &app.trigger_interval)
            .on_input(Message::TriggerIntervalChanged)
            .padding(10),]
        .into(),
        _ => column![].into(),
    };

    // Action Section
    let action_section = column![
        text("Action").size(18),
        pick_list(
            vec![
                "media".to_string(),
                "execute".to_string(),
                "powershell".to_string(),
                "log".to_string(),
            ],
            Some(app.action_type.clone()),
            Message::ActionTypeChanged,
        ),
    ];

    // Action-specific fields
    let action_fields: Element<Message> = match app.action_type.as_str() {
        "log" => column![
            text_input("Message", &app.action_message)
                .on_input(Message::ActionMessageChanged)
                .padding(10),
            pick_list(
                vec![
                    "debug".to_string(),
                    "info".to_string(),
                    "warn".to_string(),
                    "error".to_string()
                ],
                Some(app.action_log_level.clone()),
                Message::ActionLogLevelChanged,
            ),
        ]
        .spacing(5)
        .into(),
        "execute" => column![
            text_input("Command", &app.action_command)
                .on_input(Message::ActionCommandChanged)
                .padding(10),
            text_input("Arguments (space-separated)", &app.action_args)
                .on_input(Message::ActionArgsChanged)
                .padding(10),
        ]
        .spacing(5)
        .into(),
        "powershell" => column![text_input("Script", &app.action_script)
            .on_input(Message::ActionScriptChanged)
            .padding(10),]
        .into(),
        "media" => column![pick_list(
            vec![
                "play_pause".to_string(),
                "play".to_string(),
                "pause".to_string(),
                "stop".to_string(),
                "next".to_string(),
                "previous".to_string(),
                "volume_up".to_string(),
                "volume_down".to_string(),
                "mute".to_string(),
            ],
            Some(app.action_media_command.clone()),
            Message::ActionMediaCommandChanged,
        ),]
        .into(),
        _ => column![].into(),
    };

    column![
        row![
            text(title).size(24),
            Space::with_width(Length::Fill),
            button("Cancel").on_press(Message::CancelEdit),
        ]
        .spacing(20)
        .padding(20),
        scrollable(
            container(
                column![
                    basic_info,
                    trigger_section,
                    trigger_fields,
                    action_section,
                    action_fields,
                    row![
                        button("Save").on_press(Message::SaveRule(win_event_engine::RuleConfig {
                            name: app.rule_name.clone(),
                            description: None,
                            trigger: win_event_engine::TriggerConfig::Timer {
                                interval_seconds: 60
                            },
                            action: win_event_engine::ActionConfig::Log {
                                message: "Hello".to_string(),
                                level: "info".to_string(),
                            },
                            enabled: true,
                        })),
                        button("Cancel").on_press(Message::CancelEdit),
                    ]
                    .spacing(10),
                ]
                .spacing(20)
            )
            .padding(20)
        )
        .height(Length::Fill),
    ]
    .into()
}
