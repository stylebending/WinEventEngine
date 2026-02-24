use anyhow::Result;
use crossterm::{
    event::{self, DisableMouseCapture, EnableMouseCapture, Event, KeyCode, KeyEventKind},
    execute,
    terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen},
};
use ratatui::{
    backend::CrosstermBackend,
    layout::{Constraint, Direction, Layout},
    style::{Color, Style, Stylize},
    text::{Line, Span},
    widgets::{Block, Borders, Paragraph},
    Frame, Terminal,
};
use reqwest::Client;
use serde::{Deserialize, Serialize};
use std::io;
use tracing::{error, info};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RuleConfig {
    pub name: String,
    pub description: Option<String>,
    pub trigger: TriggerConfig,
    pub action: ActionConfig,
    pub enabled: bool,
}

impl Default for RuleConfig {
    fn default() -> Self {
        RuleConfig {
            name: String::new(),
            description: None,
            trigger: TriggerConfig::default(),
            action: ActionConfig::default(),
            enabled: true,
        }
    }
}

impl Default for TriggerConfig {
    fn default() -> Self {
        TriggerConfig::FileCreated { pattern: None }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum TriggerConfig {
    FileCreated { pattern: Option<String> },
    FileModified { pattern: Option<String> },
    FileDeleted { pattern: Option<String> },
    WindowFocused {
        title_contains: Option<String>,
        process_name: Option<String>,
    },
    WindowUnfocused {
        title_contains: Option<String>,
        process_name: Option<String>,
    },
    WindowCreated,
    ProcessStarted { process_name: Option<String> },
    ProcessStopped { process_name: Option<String> },
    RegistryChanged { value_name: Option<String> },
    Timer { interval_seconds: u64 },
}

impl Default for ActionConfig {
    fn default() -> Self {
        ActionConfig::Execute {
            command: String::new(),
            args: Vec::new(),
            working_dir: None,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ActionConfig {
    Execute {
        command: String,
        args: Vec<String>,
        working_dir: Option<String>,
    },
    PowerShell { script: String, working_dir: Option<String> },
    Log { message: String, level: String },
    Notify { title: String, message: String },
    HttpRequest {
        url: String,
        method: String,
        headers: std::collections::HashMap<String, String>,
        body: Option<String>,
    },
    Media { command: String },
    Script {
        path: String,
        function: String,
        timeout_ms: Option<u64>,
        on_error: String,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TestRequest {
    pub rule: RuleConfig,
    pub event: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct ApiResponse<T> {
    success: bool,
    data: Option<T>,
    error: Option<String>,
}

struct App {
    client: Client,
    base_url: String,
    rules: Vec<RuleConfig>,
    selected_rule_index: usize,
    current_tab: usize,
    message: String,
    is_editing: bool,
    editing_rule: Option<RuleConfig>,
    form_step: usize,
    form_data: RuleConfig,
    input_buffer: String,
    selected_field: usize,
    is_creating: bool,
}

impl App {
    fn new(base_url: String) -> Self {
        Self {
            client: Client::new(),
            base_url,
            rules: Vec::new(),
            selected_rule_index: 0,
            current_tab: 0,
            message: String::new(),
            is_editing: false,
            editing_rule: None,
            form_step: 0,
            form_data: RuleConfig::default(),
            input_buffer: String::new(),
            selected_field: 0,
            is_creating: false,
        }
    }

    async fn fetch_rules(&mut self) -> Result<()> {
        let url = format!("{}/api/rules", self.base_url);
        let response = self.client.get(&url).send().await?;
        
        if response.status().is_success() {
            let api_response: ApiResponse<Vec<RuleConfig>> = response.json().await?;
            if api_response.success {
                self.rules = api_response.data.unwrap_or_default();
                self.message = format!("Loaded {} rules", self.rules.len());
            } else {
                self.message = format!("Error: {}", api_response.error.unwrap_or_default());
            }
        } else {
            self.message = format!("Failed to fetch rules: {}", response.status());
        }
        Ok(())
    }

    async fn create_rule(&mut self, rule: RuleConfig) -> Result<()> {
        let url = format!("{}/api/rules", self.base_url);
        let response = self.client.post(&url).json(&rule).send().await?;
        
        if response.status().is_success() {
            let api_response: ApiResponse<RuleConfig> = response.json().await?;
            if api_response.success {
                self.message = "Rule created successfully".to_string();
                self.fetch_rules().await?;
            } else {
                self.message = format!("Error: {}", api_response.error.unwrap_or_default());
            }
        } else {
            self.message = format!("Failed to create rule: {}", response.status());
        }
        Ok(())
    }

    async fn update_rule(&mut self, name: &str, rule: RuleConfig) -> Result<()> {
        let url = format!("{}/api/rules/{}", self.base_url, name);
        let response = self.client.put(&url).json(&rule).send().await?;
        
        if response.status().is_success() {
            self.message = "Rule updated successfully".to_string();
            self.fetch_rules().await?;
        } else {
            self.message = format!("Failed to update rule: {}", response.status());
        }
        Ok(())
    }

    async fn delete_rule(&mut self, name: &str) -> Result<()> {
        let url = format!("{}/api/rules/{}", self.base_url, name);
        let response = self.client.delete(&url).send().await?;
        
        if response.status().is_success() {
            self.message = "Rule deleted successfully".to_string();
            self.fetch_rules().await?;
        } else {
            self.message = format!("Failed to delete rule: {}", response.status());
        }
        Ok(())
    }

    async fn toggle_rule(&mut self, name: &str, enabled: bool) -> Result<()> {
        let url = format!("{}/api/rules/{}/enable", self.base_url, name);
        let response = self.client.post(&url).json(&serde_json::json!({ "enabled": enabled })).send().await?;
        
        if response.status().is_success() {
            self.message = format!("Rule {} {}", name, if enabled { "enabled" } else { "disabled" });
            self.fetch_rules().await?;
        } else {
            self.message = format!("Failed to toggle rule: {}", response.status());
        }
        Ok(())
    }

    async fn export_rules(&self) -> Result<String> {
        let url = format!("{}/api/rules/export", self.base_url);
        let response = self.client.get(&url).send().await?;
        
        if response.status().is_success() {
            Ok(response.text().await?)
        } else {
            Err(anyhow::anyhow!("Export failed: {}", response.status()))
        }
    }

    async fn import_rules(&self, content: &str) -> Result<usize> {
        let url = format!("{}/api/rules/import", self.base_url);
        let response = self.client.post(&url).json(&serde_json::json!({ "content": content })).send().await?;
        
        if response.status().is_success() {
            let api_response: ApiResponse<usize> = response.json().await?;
            Ok(api_response.data.unwrap_or(0))
        } else {
            Err(anyhow::anyhow!("Import failed: {}", response.status()))
        }
    }
}

fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_max_level(tracing::Level::INFO)
        .init();

    let base_url = std::env::args()
        .nth(1)
        .unwrap_or_else(|| "http://127.0.0.1:9090".to_string());

    info!("Starting TUI with base URL: {}", base_url);

    enable_raw_mode()?;
    let mut stdout = io::stdout();
    execute!(stdout, EnterAlternateScreen, EnableMouseCapture)?;
    let backend = CrosstermBackend::new(stdout);
    let mut terminal = Terminal::new(backend)?;

    let mut app = App::new(base_url);
    
    let runtime = tokio::runtime::Runtime::new()?;
    runtime.block_on(app.fetch_rules())?;

    let res = run_app(&mut terminal, &mut app);

    disable_raw_mode()?;
    execute!(
        terminal.backend_mut(),
        LeaveAlternateScreen,
        DisableMouseCapture
    )?;
    terminal.show_cursor()?;

    if let Err(err) = res {
        error!("Error: {:?}", err);
    }

    Ok(())
}

fn run_app(terminal: &mut Terminal<CrosstermBackend<io::Stdout>>, app: &mut App) -> Result<()> {
    loop {
        terminal.draw(|f| ui(f, app))?;

        if let Event::Key(key) = event::read()? {
            if key.kind == KeyEventKind::Press {
                match key.code {
                    KeyCode::Char('q') => return Ok(()),
                    KeyCode::Char('r') => {
                        let rt = tokio::runtime::Runtime::new()?;
                        rt.block_on(app.fetch_rules())?;
                    }
                    KeyCode::Char('n') => {
                        if app.current_tab == 1 {
                            app.is_creating = true;
                            app.form_step = 0;
                            app.form_data = RuleConfig::default();
                            app.selected_field = 0;
                            app.input_buffer = String::new();
                        } else {
                            app.is_editing = true;
                            app.editing_rule = Some(RuleConfig {
                                name: "new_rule".to_string(),
                                description: None,
                                trigger: TriggerConfig::FileCreated { pattern: None },
                                action: ActionConfig::Log { message: "Hello".to_string(), level: "info".to_string() },
                                enabled: true,
                            });
                        }
                    }
                    KeyCode::Char('d') => {
                        if !app.rules.is_empty() && app.selected_rule_index < app.rules.len() && app.current_tab == 0 {
                            let rule_name = app.rules[app.selected_rule_index].name.clone();
                            let rt = tokio::runtime::Runtime::new()?;
                            rt.block_on(app.delete_rule(&rule_name))?;
                        }
                    }
                    KeyCode::Char('e') => {
                        if !app.rules.is_empty() && app.selected_rule_index < app.rules.len() && app.current_tab == 0 {
                            app.is_creating = true;
                            app.form_step = 0;
                            app.form_data = app.rules[app.selected_rule_index].clone();
                            app.selected_field = 0;
                            app.input_buffer = app.form_data.name.clone();
                        }
                    }
                    KeyCode::Char(' ') => {
                        if !app.rules.is_empty() && app.selected_rule_index < app.rules.len() && app.current_tab == 0 {
                            let rule_name = app.rules[app.selected_rule_index].name.clone();
                            let enabled = app.rules[app.selected_rule_index].enabled;
                            let rt = tokio::runtime::Runtime::new()?;
                            rt.block_on(app.toggle_rule(&rule_name, !enabled))?;
                        }
                    }
                    KeyCode::Down => {
                        if app.is_creating && app.current_tab == 1 {
                            let max_fields = match app.form_step {
                                0 => 2,
                                1 => 11,
                                2 => 9,
                                3 => 0,
                                _ => 0,
                            };
                            if app.selected_field < max_fields {
                                app.selected_field += 1;
                            }
                        } else if app.selected_rule_index < app.rules.len().saturating_sub(1) {
                            app.selected_rule_index += 1;
                        }
                    }
                    KeyCode::Up => {
                        if app.is_creating && app.current_tab == 1 {
                            if app.selected_field > 0 {
                                app.selected_field -= 1;
                            }
                        } else if app.selected_rule_index > 0 {
                            app.selected_rule_index -= 1;
                        }
                    }
                    KeyCode::Tab => {
                        app.current_tab = (app.current_tab + 1) % 4;
                    }
                    KeyCode::Char('1') => app.current_tab = 0,
                    KeyCode::Char('2') => app.current_tab = 1,
                    KeyCode::Char('3') => app.current_tab = 2,
                    KeyCode::Char('4') => app.current_tab = 3,
                    KeyCode::Right => {
                        if app.is_creating && app.current_tab == 1 && app.form_step < 3 {
                            app.form_step += 1;
                            app.selected_field = 0;
                        }
                    }
                    KeyCode::Left => {
                        if app.is_creating && app.current_tab == 1 && app.form_step > 0 {
                            app.form_step -= 1;
                            app.selected_field = 0;
                        }
                    }
                    KeyCode::Enter => {
                        if app.is_creating && app.current_tab == 1 {
                            if app.form_step < 3 {
                                app.form_step += 1;
                                app.selected_field = 0;
                            } else {
                                let rt = tokio::runtime::Runtime::new()?;
                                rt.block_on(app.create_rule(app.form_data.clone()))?;
                                app.is_creating = false;
                                app.form_data = RuleConfig::default();
                                app.form_step = 0;
                                app.current_tab = 0;
                            }
                        } else if app.is_editing {
                            if let Some(ref rule) = app.editing_rule {
                                let rule_name = rule.name.clone();
                                let rule_clone = rule.clone();
                                let rt = tokio::runtime::Runtime::new()?;
                                if app.rules.iter().any(|r| r.name == rule.name) {
                                    rt.block_on(app.update_rule(&rule_name, rule_clone))?;
                                } else {
                                    rt.block_on(app.create_rule(rule_clone))?;
                                }
                                app.is_editing = false;
                                app.editing_rule = None;
                            }
                        }
                    }
                    KeyCode::Esc => {
                        if app.is_creating {
                            app.is_creating = false;
                            app.form_data = RuleConfig::default();
                            app.form_step = 0;
                            app.selected_field = 0;
                        }
                        app.is_editing = false;
                        app.editing_rule = None;
                    }
                    KeyCode::Char(c) => {
                        if app.is_creating && app.current_tab == 1 {
                            match app.form_step {
                                0 => {
                                    if app.selected_field == 0 {
                                        app.form_data.name.push(c);
                                    } else {
                                        if app.form_data.description.is_none() {
                                            app.form_data.description = Some(String::new());
                                        }
                                        if let Some(desc) = &mut app.form_data.description {
                                            desc.push(c);
                                        }
                                    }
                                }
                                1 => {
                                    let trigger_idx = app.selected_field;
                                    if trigger_idx < 10 {
                                        app.form_data.trigger = match trigger_idx {
                                            0 => TriggerConfig::FileCreated { pattern: None },
                                            1 => TriggerConfig::FileModified { pattern: None },
                                            2 => TriggerConfig::FileDeleted { pattern: None },
                                            3 => TriggerConfig::WindowFocused { title_contains: None, process_name: None },
                                            4 => TriggerConfig::WindowUnfocused { title_contains: None, process_name: None },
                                            5 => TriggerConfig::WindowCreated,
                                            6 => TriggerConfig::ProcessStarted { process_name: None },
                                            7 => TriggerConfig::ProcessStopped { process_name: None },
                                            8 => TriggerConfig::RegistryChanged { value_name: None },
                                            9 => TriggerConfig::Timer { interval_seconds: 60 },
                                            _ => TriggerConfig::FileCreated { pattern: None },
                                        };
                                    } else if trigger_idx == 10 {
                                        match &mut app.form_data.trigger {
                                            TriggerConfig::FileCreated { pattern } | TriggerConfig::FileModified { pattern } | TriggerConfig::FileDeleted { pattern } => {
                                                if pattern.is_none() {
                                                    *pattern = Some(String::new());
                                                }
                                                if let Some(p) = pattern {
                                                    p.push(c);
                                                }
                                            }
                                            TriggerConfig::WindowFocused { title_contains, process_name } | TriggerConfig::WindowUnfocused { title_contains, process_name } => {
                                                if app.selected_field == 10 {
                                                    if title_contains.is_none() {
                                                        *title_contains = Some(String::new());
                                                    }
                                                    if let Some(t) = title_contains {
                                                        t.push(c);
                                                    }
                                                } else {
                                                    if process_name.is_none() {
                                                        *process_name = Some(String::new());
                                                    }
                                                    if let Some(p) = process_name {
                                                        p.push(c);
                                                    }
                                                }
                                            }
                                            TriggerConfig::ProcessStarted { process_name } | TriggerConfig::ProcessStopped { process_name } => {
                                                if process_name.is_none() {
                                                    *process_name = Some(String::new());
                                                }
                                                if let Some(p) = process_name {
                                                    p.push(c);
                                                }
                                            }
                                            TriggerConfig::RegistryChanged { value_name } => {
                                                if value_name.is_none() {
                                                    *value_name = Some(String::new());
                                                }
                                                if let Some(v) = value_name {
                                                    v.push(c);
                                                }
                                            }
                                            TriggerConfig::Timer { interval_seconds } => {
                                                if let Some(digit) = c.to_digit(10) {
                                                    *interval_seconds = *interval_seconds * 10 + digit as u64;
                                                }
                                            }
                                            TriggerConfig::WindowCreated { .. } => {}
                                        }
                                    }
                                }
                                2 => {
                                    let action_idx = app.selected_field;
                                    if action_idx < 6 {
                                        app.form_data.action = match action_idx {
                                            0 => ActionConfig::Execute { command: String::new(), args: Vec::new(), working_dir: None },
                                            1 => ActionConfig::PowerShell { script: String::new(), working_dir: None },
                                            2 => ActionConfig::Log { message: String::new(), level: "info".to_string() },
                                            3 => ActionConfig::Notify { title: String::new(), message: String::new() },
                                            4 => ActionConfig::HttpRequest { url: String::new(), method: "GET".to_string(), headers: std::collections::HashMap::new(), body: None },
                                            5 => ActionConfig::Media { command: "play".to_string() },
                                            _ => ActionConfig::Log { message: String::new(), level: "info".to_string() },
                                        };
                                    }
                                }
                                _ => {}
                            }
                        }
                    }
                    KeyCode::Backspace => {
                        if app.is_creating && app.current_tab == 1 {
                            match app.form_step {
                                0 => {
                                    if app.selected_field == 0 {
                                        app.form_data.name.pop();
                                    } else {
                                        if let Some(desc) = &mut app.form_data.description {
                                            desc.pop();
                                        }
                                    }
                                }
                                1 => {
                                    if app.selected_field == 10 {
                                        match &mut app.form_data.trigger {
                                            TriggerConfig::FileCreated { pattern } | TriggerConfig::FileModified { pattern } | TriggerConfig::FileDeleted { pattern } => {
                                                if let Some(p) = pattern {
                                                    p.pop();
                                                }
                                            }
                                            TriggerConfig::WindowFocused { title_contains, process_name } => {
                                                if let Some(t) = title_contains {
                                                    t.pop();
                                                }
                                            }
                                            TriggerConfig::WindowUnfocused { title_contains, process_name } => {
                                                if let Some(t) = title_contains {
                                                    t.pop();
                                                }
                                            }
                                            TriggerConfig::ProcessStarted { process_name } | TriggerConfig::ProcessStopped { process_name } => {
                                                if let Some(p) = process_name {
                                                    p.pop();
                                                }
                                            }
                                            TriggerConfig::RegistryChanged { value_name } => {
                                                if let Some(v) = value_name {
                                                    v.pop();
                                                }
                                            }
                                            TriggerConfig::Timer { interval_seconds } => {
                                                *interval_seconds /= 10;
                                            }
                                            TriggerConfig::WindowCreated { .. } => {}
                                        }
                                    }
                                }
                                _ => {}
                            }
                        }
                    }
                    _ => {}
                }
            }
        }
    }
}

fn ui(frame: &mut Frame, app: &App) {
    // Main layout: sidebar (left) + content (right)
    let main_chunks = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([
            Constraint::Length(24),
            Constraint::Min(0),
        ])
        .split(frame.area());

    // Sidebar with tabs
    let sidebar = render_sidebar(app);
    frame.render_widget(sidebar, main_chunks[0]);

    // Content area
    let content_chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(3),
            Constraint::Min(0),
            Constraint::Length(3),
        ])
        .split(main_chunks[1]);

    // Tab bar
    let tab_bar = render_tab_bar(app);
    frame.render_widget(tab_bar, content_chunks[0]);

    // Main content
    match app.current_tab {
        0 => frame.render_widget(render_list_view(app), content_chunks[1]),
        1 => frame.render_widget(render_create_view(app), content_chunks[1]),
        2 => frame.render_widget(render_test_view(app), content_chunks[1]),
        3 => frame.render_widget(render_import_export_view(app), content_chunks[1]),
        _ => {}
    }

    // Status bar
    let status_bar = Paragraph::new(app.message.clone())
        .style(Style::default().fg(if app.message.contains("Error") { Color::Red } else { Color::Green }))
        .block(Block::default().borders(Borders::ALL).title("Status"));
    frame.render_widget(status_bar, content_chunks[2]);
}

fn render_sidebar(app: &App) -> Paragraph<'static> {
    let mut content = String::from("\n WinEventEngine\n");
    content.push_str(" ─────────────────\n");
    content.push_str("\n Keybindings:\n");
    content.push_str(" ─────────────────\n");
    content.push_str(" q  - Quit\n");
    content.push_str(" r  - Refresh\n");
    content.push_str(" n  - New rule\n");
    content.push_str(" e  - Edit\n");
    content.push_str(" d  - Delete\n");
    content.push_str(" ␣  - Toggle\n");
    content.push_str(" ↑↓  - Navigate\n");
    content.push_str(" 1-4 - Switch tab\n");
    content.push_str(" Ent - Save\n");
    content.push_str(" Esc - Cancel\n");

    Paragraph::new(content)
        .style(Style::default().fg(Color::White))
        .block(Block::default().borders(Borders::ALL).title("Menu").border_style(Style::default().fg(Color::Blue)))
}

fn render_tab_bar(app: &App) -> Paragraph<'static> {
    let tabs = ["Automations", "Create Rule", "Test Rule", "Import/Export"];
    let tab_names: Vec<Line> = tabs
        .iter()
        .enumerate()
        .map(|(idx, t)| {
            if idx == app.current_tab {
                Line::from(vec![
                    Span::styled(" [", Style::default().fg(Color::DarkGray)),
                    Span::styled((idx + 1).to_string(), Style::default().fg(Color::Cyan).bold()),
                    Span::styled("] ", Style::default().fg(Color::DarkGray)),
                    Span::styled(*t, Style::default().fg(Color::White).bold()),
                ])
            } else {
                Line::from(vec![
                    Span::styled(" [", Style::default().fg(Color::DarkGray)),
                    Span::styled((idx + 1).to_string(), Style::default().fg(Color::DarkGray)),
                    Span::styled("] ", Style::default().fg(Color::DarkGray)),
                    Span::styled(*t, Style::default().fg(Color::DarkGray)),
                ])
            }
        })
        .collect();

    Paragraph::new(tab_names)
        .block(Block::default().borders(Borders::ALL).title("Tabs").border_style(Style::default().fg(Color::Blue)))
        .alignment(ratatui::layout::Alignment::Center)
}

fn render_list_view(app: &App) -> Paragraph<'static> {
    if app.rules.is_empty() {
        return Paragraph::new("No rules found.\n\nPress 'n' to create one or 'r' to refresh.")
            .style(Style::default().fg(Color::Gray))
            .block(Block::default().title("Automations").borders(Borders::ALL).border_style(Style::default().fg(Color::Blue)));
    }

    let mut content = String::from(" Status   │ Name                 │ Trigger              │ Action\n");
    content.push_str("──────────┼──────────────────────┼──────────────────────┼─────────────────────\n");
    
    for (idx, rule) in app.rules.iter().enumerate() {
        let status = if rule.enabled { "  ✓  " } else { "  ✗  " };
        let trigger_type = format!("{:?}", rule.trigger).split('{').next().unwrap_or("?").chars().take(20).collect::<String>();
        let action_type = format!("{:?}", rule.action).split('{').next().unwrap_or("?").chars().take(20).collect::<String>();
        
        let prefix = if idx == app.selected_rule_index { "►" } else { " " };
        
        let line = format!("{} {} │ {:20} │ {:20} │ {:20}\n", 
            prefix,
            status, 
            rule.name.chars().take(20).collect::<String>(),
            trigger_type,
            action_type
        );
        content.push_str(&line);
    }

    let style = Style::default();
    if app.selected_rule_index < app.rules.len() {
        Paragraph::new(content)
            .style(style)
            .block(Block::default().title("Automations - Space=toggle, n=new, d=delete, r=refresh").borders(Borders::ALL).border_style(Style::default().fg(Color::Blue)))
    } else {
        Paragraph::new(content)
            .style(style.fg(Color::White))
            .block(Block::default().title("Automations").borders(Borders::ALL).border_style(Style::default().fg(Color::Blue)))
    }
}

fn render_create_view(app: &App) -> Paragraph<'static> {
    if !app.is_creating && app.editing_rule.is_none() {
        return Paragraph::new("Press 'n' to create a new rule")
            .style(Style::default().fg(Color::Gray))
            .block(Block::default().title("Create Rule").borders(Borders::ALL).border_style(Style::default().fg(Color::Blue)));
    }

    let step_names = ["Basic Info", "Trigger", "Action", "Review"];
    let current_step_name = step_names.get(app.form_step).unwrap_or(&"Unknown");
    
    let mut content = format!("\n━━━ Step {}/4: {} ━━━\n\n", app.form_step + 1, current_step_name);
    
    match app.form_step {
        0 => {
            content.push_str("Rule Name:\n");
            let name = &app.form_data.name;
            let prefix = if app.selected_field == 0 { "► " } else { "  " };
            content.push_str(&format!("{}{}\n\n", prefix, if name.is_empty() { "(required)" } else { name }));
            
            content.push_str("Description (optional):\n");
            let desc = app.form_data.description.as_deref().unwrap_or("");
            let prefix = if app.selected_field == 1 { "► " } else { "  " };
            content.push_str(&format!("{}{}\n", prefix, if desc.is_empty() { "(optional)" } else { desc }));
        }
        1 => {
            content.push_str("Trigger Type:\n");
            let trigger_types = [
                ("file_created", "File Created"),
                ("file_modified", "File Modified"),
                ("file_deleted", "File Deleted"),
                ("window_focused", "Window Focused"),
                ("window_unfocused", "Window Unfocused"),
                ("window_created", "Window Created"),
                ("process_started", "Process Started"),
                ("process_stopped", "Process Stopped"),
                ("registry_changed", "Registry Changed"),
                ("timer", "Timer"),
            ];
            
            let current_trigger = match &app.form_data.trigger {
                TriggerConfig::FileCreated { .. } => "file_created",
                TriggerConfig::FileModified { .. } => "file_modified",
                TriggerConfig::FileDeleted { .. } => "file_deleted",
                TriggerConfig::WindowFocused { .. } => "window_focused",
                TriggerConfig::WindowUnfocused { .. } => "window_unfocused",
                TriggerConfig::WindowCreated { .. } => "window_created",
                TriggerConfig::ProcessStarted { .. } => "process_started",
                TriggerConfig::ProcessStopped { .. } => "process_stopped",
                TriggerConfig::RegistryChanged { .. } => "registry_changed",
                TriggerConfig::Timer { .. } => "timer",
            };
            
            for (i, (id, name)) in trigger_types.iter().enumerate() {
                let prefix = if app.selected_field == i { "►" } else { " " };
                let mark = if *id == current_trigger { "●" } else { "○" };
                content.push_str(&format!("{} {} {}\n", prefix, mark, name));
            }
            
            content.push_str("\nTrigger Config:\n");
            match &app.form_data.trigger {
                TriggerConfig::FileCreated { pattern } | TriggerConfig::FileModified { pattern } | TriggerConfig::FileDeleted { pattern } => {
                    let p = pattern.as_deref().unwrap_or("");
                    let prefix = if app.selected_field == 10 { "► " } else { "  " };
                    content.push_str(&format!("{}Pattern: {}\n", prefix, if p.is_empty() { "(e.g., C:/logs/*.txt)" } else { p }));
                }
                TriggerConfig::WindowFocused { title_contains, process_name } | TriggerConfig::WindowUnfocused { title_contains, process_name } => {
                    let t = title_contains.as_deref().unwrap_or("");
                    let p = process_name.as_deref().unwrap_or("");
                    let prefix1 = if app.selected_field == 10 { "► " } else { "  " };
                    let prefix2 = if app.selected_field == 11 { "► " } else { "  " };
                    content.push_str(&format!("{}Title contains: {}\n", prefix1, if t.is_empty() { "(optional)" } else { t }));
                    content.push_str(&format!("{}Process: {}\n", prefix2, if p.is_empty() { "(optional)" } else { p }));
                }
                TriggerConfig::ProcessStarted { process_name } | TriggerConfig::ProcessStopped { process_name } => {
                    let p = process_name.as_deref().unwrap_or("");
                    let prefix = if app.selected_field == 10 { "► " } else { "  " };
                    content.push_str(&format!("{}Process name: {}\n", prefix, if p.is_empty() { "(e.g., notepad.exe)" } else { p }));
                }
                TriggerConfig::RegistryChanged { value_name } => {
                    let v = value_name.as_deref().unwrap_or("");
                    let prefix = if app.selected_field == 10 { "► " } else { "  " };
                    content.push_str(&format!("{}Value name: {}\n", prefix, if v.is_empty() { "(e.g., HKCU/Software/MyApp)" } else { v }));
                }
                TriggerConfig::Timer { interval_seconds } => {
                    let prefix = if app.selected_field == 10 { "► " } else { "  " };
                    content.push_str(&format!("{}Interval (seconds): {}\n", prefix, interval_seconds));
                }
                TriggerConfig::WindowCreated { .. } => {
                    content.push_str("  (no config needed)\n");
                }
            }
        }
        2 => {
            content.push_str("Action Type:\n");
            let action_types = [
                ("execute", "Execute Command"),
                ("powershell", "PowerShell Script"),
                ("log", "Log Message"),
                ("notify", "Show Notification"),
                ("http_request", "HTTP Request"),
                ("media", "Media Control"),
            ];
            
            let current_action = match &app.form_data.action {
                ActionConfig::Execute { .. } => "execute",
                ActionConfig::PowerShell { .. } => "powershell",
                ActionConfig::Log { .. } => "log",
                ActionConfig::Notify { .. } => "notify",
                ActionConfig::HttpRequest { .. } => "http_request",
                ActionConfig::Media { .. } => "media",
                ActionConfig::Script { .. } => "script",
            };
            
            for (i, (id, name)) in action_types.iter().enumerate() {
                let prefix = if app.selected_field == i { "►" } else { " " };
                let mark = if *id == current_action { "●" } else { "○" };
                content.push_str(&format!("{} {} {}\n", prefix, mark, name));
            }
            
            content.push_str("\nAction Config:\n");
            match &app.form_data.action {
                ActionConfig::Execute { command, args, working_dir } => {
                    let c = command.as_str();
                    let a = args.join(" ");
                    let w = working_dir.as_deref().unwrap_or("");
                    let prefix1 = if app.selected_field == 6 { "► " } else { "  " };
                    let prefix2 = if app.selected_field == 7 { "► " } else { "  " };
                    let prefix3 = if app.selected_field == 8 { "► " } else { "  " };
                    content.push_str(&format!("{}Command: {}\n", prefix1, if c.is_empty() { "(required)" } else { c }));
                    content.push_str(&format!("{}Args: {}\n", prefix2, if a.is_empty() { "(optional)" } else { &a }));
                    content.push_str(&format!("{}Working dir: {}\n", prefix3, if w.is_empty() { "(optional)" } else { w }));
                }
                ActionConfig::PowerShell { script, working_dir } => {
                    let s = script.as_str();
                    let w = working_dir.as_deref().unwrap_or("");
                    let prefix1 = if app.selected_field == 6 { "► " } else { "  " };
                    let prefix2 = if app.selected_field == 7 { "► " } else { "  " };
                    content.push_str(&format!("{}Script: {}\n", prefix1, if s.is_empty() { "(required)" } else { s }));
                    content.push_str(&format!("{}Working dir: {}\n", prefix2, if w.is_empty() { "(optional)" } else { w }));
                }
                ActionConfig::Log { message, level } => {
                    let m = message.as_str();
                    let prefix1 = if app.selected_field == 6 { "► " } else { "  " };
                    let prefix2 = if app.selected_field == 7 { "► " } else { "  " };
                    content.push_str(&format!("{}Message: {}\n", prefix1, if m.is_empty() { "(required)" } else { m }));
                    content.push_str(&format!("{}Level: {}\n", prefix2, level));
                }
                ActionConfig::Notify { title, message } => {
                    let t = title.as_str();
                    let m = message.as_str();
                    let prefix1 = if app.selected_field == 6 { "► " } else { "  " };
                    let prefix2 = if app.selected_field == 7 { "► " } else { "  " };
                    content.push_str(&format!("{}Title: {}\n", prefix1, if t.is_empty() { "(required)" } else { t }));
                    content.push_str(&format!("{}Message: {}\n", prefix2, if m.is_empty() { "(required)" } else { m }));
                }
                ActionConfig::HttpRequest { url, method, body, .. } => {
                    let u = url.as_str();
                    let m = method.as_str();
                    let b = body.as_deref().unwrap_or("");
                    let prefix1 = if app.selected_field == 6 { "► " } else { "  " };
                    let prefix2 = if app.selected_field == 7 { "► " } else { "  " };
                    let prefix3 = if app.selected_field == 8 { "► " } else { "  " };
                    content.push_str(&format!("{}URL: {}\n", prefix1, if u.is_empty() { "(required)" } else { u }));
                    content.push_str(&format!("{}Method: {}\n", prefix2, m));
                    content.push_str(&format!("{}Body: {}\n", prefix3, if b.is_empty() { "(optional)" } else { b }));
                }
                ActionConfig::Media { command } => {
                    let c = command.as_str();
                    let prefix = if app.selected_field == 6 { "► " } else { "  " };
                    content.push_str(&format!("{}Command: {}\n", prefix, if c.is_empty() { "(play, pause, next, prev, mute, vol_up, vol_down)" } else { c }));
                }
                ActionConfig::Script { path, function, timeout_ms, on_error } => {
                    let p = path.as_str();
                    let f = function.as_str();
                    let t = timeout_ms.map(|t| t.to_string()).unwrap_or_default();
                    let o = on_error.as_str();
                    let prefix1 = if app.selected_field == 6 { "► " } else { "  " };
                    let prefix2 = if app.selected_field == 7 { "► " } else { "  " };
                    let prefix3 = if app.selected_field == 8 { "► " } else { "  " };
                    let prefix4 = if app.selected_field == 9 { "► " } else { "  " };
                    content.push_str(&format!("{}Path: {}\n", prefix1, if p.is_empty() { "(required)" } else { p }));
                    content.push_str(&format!("{}Function: {}\n", prefix2, if f.is_empty() { "(required)" } else { f }));
                    content.push_str(&format!("{}Timeout (ms): {}\n", prefix3, if t.is_empty() { "(optional)" } else { &t }));
                    content.push_str(&format!("{}On error: {}\n", prefix4, if o.is_empty() { "(continue)" } else { o }));
                }
            }
        }
        3 => {
            content.push_str("━━━ Review Your Rule ━━━\n\n");
            content.push_str(&format!("Name: {}\n", app.form_data.name));
            if let Some(desc) = &app.form_data.description {
                content.push_str(&format!("Description: {}\n", desc));
            }
            content.push_str(&format!("Enabled: {}\n\n", if app.form_data.enabled { "Yes" } else { "No" }));
            
            content.push_str("Trigger:\n");
            match &app.form_data.trigger {
                TriggerConfig::FileCreated { pattern } => content.push_str(&format!("  Type: File Created\n  Pattern: {:?}\n", pattern)),
                TriggerConfig::FileModified { pattern } => content.push_str(&format!("  Type: File Modified\n  Pattern: {:?}\n", pattern)),
                TriggerConfig::FileDeleted { pattern } => content.push_str(&format!("  Type: File Deleted\n  Pattern: {:?}\n", pattern)),
                TriggerConfig::WindowFocused { title_contains, process_name } => content.push_str(&format!("  Type: Window Focused\n  Title: {:?}\n  Process: {:?}\n", title_contains, process_name)),
                TriggerConfig::WindowUnfocused { title_contains, process_name } => content.push_str(&format!("  Type: Window Unfocused\n  Title: {:?}\n  Process: {:?}\n", title_contains, process_name)),
                TriggerConfig::WindowCreated { .. } => content.push_str("  Type: Window Created\n"),
                TriggerConfig::ProcessStarted { process_name } => content.push_str(&format!("  Type: Process Started\n  Process: {:?}\n", process_name)),
                TriggerConfig::ProcessStopped { process_name } => content.push_str(&format!("  Type: Process Stopped\n  Process: {:?}\n", process_name)),
                TriggerConfig::RegistryChanged { value_name } => content.push_str(&format!("  Type: Registry Changed\n  Value: {:?}\n", value_name)),
                TriggerConfig::Timer { interval_seconds } => content.push_str(&format!("  Type: Timer\n  Interval: {}s\n", interval_seconds)),
            }
            
            content.push_str("\nAction:\n");
            match &app.form_data.action {
                ActionConfig::Execute { command, args, .. } => content.push_str(&format!("  Type: Execute\n  Command: {}\n  Args: {:?}\n", command, args)),
                ActionConfig::PowerShell { script, .. } => content.push_str(&format!("  Type: PowerShell\n  Script: {}\n", script)),
                ActionConfig::Log { message, level } => content.push_str(&format!("  Type: Log\n  Level: {}\n  Message: {}\n", level, message)),
                ActionConfig::Notify { title, message } => content.push_str(&format!("  Type: Notify\n  Title: {}\n  Message: {}\n", title, message)),
                ActionConfig::HttpRequest { url, method, .. } => content.push_str(&format!("  Type: HTTP {}\n  URL: {}\n", method, url)),
                ActionConfig::Media { command } => content.push_str(&format!("  Type: Media\n  Command: {}\n", command)),
                ActionConfig::Script { path, function, .. } => content.push_str(&format!("  Type: Script\n  Path: {}\n  Function: {}\n", path, function)),
            }
            
            content.push_str("\nPress Enter to save, Esc to cancel");
        }
        _ => {}
    }
    
    Paragraph::new(content)
        .style(Style::default().fg(Color::White))
        .block(Block::default().title("Create Rule").borders(Borders::ALL).border_style(Style::default().fg(Color::Blue)))
}

fn render_test_view(app: &App) -> Paragraph<'static> {
    let content: String = if app.rules.is_empty() {
        "No rules available for testing.\nCreate some rules first!".to_string()
    } else {
        let rule = &app.rules[app.selected_rule_index];
        let event_json = r#"{
    "id": "00000000-0000-0000-0000-000000000000",
    "timestamp": "2024-01-01T00:00:00Z",
    "kind": { "FileCreated": { "path": "C:/test.txt" } },
    "source": "test",
    "metadata": {}
}"#;
        
        format!(
            "Testing rule: {}\n\nSample Event JSON:\n{}\n\nUse the API to test with real events.",
            rule.name,
            event_json
        )
    };
    
    Paragraph::new(content)
        .style(Style::default().fg(Color::White))
        .block(Block::default().title("Test Rule").borders(Borders::ALL).border_style(Style::default().fg(Color::Blue)))
}

fn render_import_export_view(_app: &App) -> Paragraph<'static> {
    let content = "Import/Export Automations\n\n\n\
Use the web dashboard at http://127.0.0.1:9090/import-export\nfor full import/export functionality.\n\n\
Export: GET /api/rules/export\nImport: POST /api/rules/import";
    
    Paragraph::new(content)
        .style(Style::default().fg(Color::White))
        .block(Block::default().title("Import/Export").borders(Borders::ALL).border_style(Style::default().fg(Color::Blue)))
}
