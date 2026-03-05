use iced::widget::{column, container, row, scrollable, text, Space};
use iced::{Element, Length};

use crate::app::{Message, WinEventApp};

pub fn view(app: &WinEventApp) -> Element<Message> {
    let event_stream: Element<Message> = if app.events.is_empty() {
        text("No events yet...").into()
    } else {
        scrollable(
            column(
                app.events
                    .iter()
                    .map(|e| {
                        row![
                            text(&e.timestamp).width(100),
                            text(&e.source).width(150),
                            text(&e.event_type).width(200),
                            text(&e.details),
                        ]
                        .spacing(10)
                        .into()
                    })
                    .collect::<Vec<_>>(),
            )
            .spacing(5),
        )
        .into()
    };

    column![
        // Header
        row![
            text("Dashboard").size(28),
            Space::with_width(Length::Fill),
            text(format!(
                "Plugins: {} | Rules: {}",
                app.engine_status.active_plugins, app.engine_status.active_rules
            )),
        ]
        .spacing(20)
        .padding(20),
        // Metrics cards with real data
        row![
            metric_card("Events Total", app.metrics_events_total),
            metric_card("Rules Matched", app.metrics_rules_matched),
            metric_card("Actions", app.metrics_actions_executed),
        ]
        .spacing(20)
        .padding(20),
        // Event stream
        container(column![text("Event Stream").size(20), event_stream].spacing(10))
            .padding(20)
            .height(Length::Fill),
    ]
    .into()
}

fn metric_card(title: &str, value: u64) -> Element<Message> {
    container(
        column![text(title).size(14), text(format!("{}", value)).size(32),]
            .spacing(5)
            .padding(20),
    )
    .into()
}
