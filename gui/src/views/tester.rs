use iced::widget::{
    button, column, container, pick_list, row, scrollable, text, text_editor, Space,
};
use iced::{Element, Length, Theme};

use crate::app::{Message, WinEventApp};

pub fn view(app: &WinEventApp) -> Element<Message> {
    // Rule selection dropdown
    let rule_options: Vec<String> = app.rules.iter().map(|r| r.name.clone()).collect();

    let rule_selector = row![
        text("Select Rule:").width(100),
        pick_list(
            rule_options,
            if app.test_rule_name.is_empty() {
                None
            } else {
                Some(app.test_rule_name.clone())
            },
            Message::TestRuleChanged,
        )
        .width(Length::Fill),
    ]
    .spacing(10);

    // Event JSON input using text_editor for multiline support
    let json_label = text("Event JSON:").size(16);

    let json_editor = text_editor(&app.test_event_content)
        .on_action(Message::TestEventJsonAction)
        .placeholder("Paste event JSON here...")
        .padding(10)
        .font(iced::Font::MONOSPACE);

    let json_container = container(json_editor)
        .width(Length::Fill)
        .height(Length::Fixed(200.0))
        .padding(10)
        .style(|theme: &Theme| container::Style {
            background: Some(theme.palette().background.into()),
            border: iced::Border {
                color: theme.palette().primary.into(),
                width: 1.0,
                radius: 4.0.into(),
            },
            ..Default::default()
        });

    // Test button
    let test_btn = button("Test Rule")
        .on_press(Message::RunEventTest)
        .padding(10);

    // Results display
    let results = match &app.test_result {
        Some((matched, details)) => {
            let (result_text, result_color) = if *matched {
                ("✓ RULE MATCHED", iced::Color::from_rgb(0.2, 0.8, 0.2))
            } else {
                ("✗ Rule did not match", iced::Color::from_rgb(0.9, 0.2, 0.2))
            };

            container(
                column![
                    text(result_text)
                        .size(18)
                        .style(move |_theme: &Theme| text::Style {
                            color: Some(result_color),
                            ..Default::default()
                        }),
                    text(details).size(14),
                ]
                .spacing(10),
            )
            .padding(15)
            .style(move |_theme: &Theme| container::Style {
                background: Some(result_color.scale_alpha(0.1).into()),
                border: iced::Border {
                    color: result_color.into(),
                    width: 1.0,
                    radius: 6.0.into(),
                },
                ..Default::default()
            })
        }
        None => container(text("Results will appear here after testing...").style(
            |theme: &Theme| text::Style {
                color: Some(theme.palette().text.scale_alpha(0.5)),
                ..Default::default()
            },
        ))
        .padding(15),
    };

    let tester_content = column![
        text("Test automation rules against sample events").size(14),
        rule_selector,
        json_label,
        json_container,
        test_btn,
        results,
    ]
    .spacing(20);

    column![
        // Header
        row![
            text("Event Tester").size(28),
            Space::with_width(Length::Fill),
        ]
        .padding(20),
        // Test form
        container(tester_content).padding(20).height(Length::Fill),
    ]
    .into()
}
