//! Debugging through Godot's debug adapter (DAP): the connection as an
//! iced subscription, the session (paused or running, call stack,
//! variables), and the DEBUG panel.
//!
//! Godot answers `variables` with an error until it has fetched the frame
//! from the game, so failed requests are retried shortly after.

use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};
use std::path::{Path, PathBuf};

use iced::futures::channel::mpsc;
use iced::futures::{SinkExt, StreamExt};
use iced::widget::tooltip::Position;
use iced::widget::{button, column, container, row, scrollable, text, text_input, Column};
use iced::{Alignment, Background, Color, Element, Font, Length, Subscription};
use lsp_client::dap::{self, DapClient, DapMessage, Scope, StackFrame, Variable};
use editor_core::Pos;
use serde_json::Value;

use crate::style;

#[derive(Debug, Clone)]
pub enum DebugEvent {
    Connected(DapClient),
    Message(DapMessage),
    Failed(String),
    Closed,
}

#[derive(Debug, Clone)]
pub enum DebugMessage {
    SelectFrame(usize),
    /// Expand or collapse a scope or variable (by its path in the tree).
    Toggle(String, i64),
    /// `continue`, `next`, `stepIn`, `stepOut` or `pause`.
    Step(&'static str),
    Stop,
    /// The text of the watch being typed.
    WatchInput(String),
    /// Add the typed watch.
    AddWatch,
    RemoveWatch(usize),
}

/// Identifies a connection: a new session gets a new generation.
#[derive(Debug, Clone, Hash)]
struct Connection {
    generation: u64,
    port: u16,
}

pub fn connect(generation: u64, port: u16) -> Subscription<(u64, DebugEvent)> {
    Subscription::run_with(Connection { generation, port }, |connection| {
        let Connection { generation, port } = connection.clone();
        iced::stream::channel(64, async move |mut output| {
            let (tx, mut rx) = mpsc::unbounded();
            std::thread::spawn(move || match dap::connect(port) {
                Ok((client, reader)) => {
                    if tx.unbounded_send(DebugEvent::Connected(client)).is_ok() {
                        reader.run(|message| drop(tx.unbounded_send(DebugEvent::Message(message))));
                    }
                    let _ = tx.unbounded_send(DebugEvent::Closed);
                }
                Err(error) => drop(tx.unbounded_send(DebugEvent::Failed(error))),
            });
            while let Some(event) = rx.next().await {
                if output.send((generation, event)).await.is_err() {
                    break;
                }
            }
        })
    })
}

#[derive(Debug, Clone, PartialEq)]
pub enum State {
    Connecting,
    Running,
    /// Why it stopped ("Breakpoint", "Step", "Exception"), and an
    /// exception's message.
    Paused { reason: String, text: Option<String> },
}

/// What the app should do after a debugger message.
#[derive(Debug, Clone, PartialEq)]
pub enum Effect {
    /// Send every breakpoint, then `configurationDone`.
    Configure,
    /// Show this (zero-based) line, paused there.
    Jump(PathBuf, usize),
    /// Game output; `true` for errors.
    Output(String, bool),
    /// Ask for these variables again in a moment.
    Retry(i64, u32),
    /// The session is over (with why, if it failed).
    Ended(Option<String>),
    /// The value of the expression hovered at this position.
    HoverValue(Pos, String, String),
}

enum Request {
    StackTrace,
    Scopes,
    Variables { reference: i64, attempt: u32 },
    Watch(String),
    Hover(Pos, String),
}

/// Tries for a frame's variables before giving up (about a second).
const MAX_ATTEMPTS: u32 = 10;

pub struct Session {
    pub generation: u64,
    pub port: u16,
    /// The `res://` scene launched, or `None` for the main scene.
    pub scene: Option<String>,
    pub client: Option<DapClient>,
    pub state: State,
    pub frames: Vec<StackFrame>,
    pub frame: usize,
    pub scopes: Vec<Scope>,
    /// Loaded children, by variables reference.
    children: HashMap<i64, Vec<Variable>>,
    /// Tree path ("Locals/self") of each reference, for the children's paths.
    paths: HashMap<i64, String>,
    /// Expanded tree paths; kept from one pause to the next.
    expanded: HashSet<String>,
    pending: HashMap<i64, Request>,
    /// Watch expressions (the app keeps them in the session), their
    /// values at this pause (text and variables reference), and the one
    /// being typed.
    pub watches: Vec<String>,
    watch_values: HashMap<String, (String, i64)>,
    pub watch_input: String,
}

impl Session {
    pub fn new(generation: u64, port: u16, scene: Option<String>, watches: Vec<String>) -> Self {
        Self {
            generation,
            port,
            scene,
            client: None,
            state: State::Connecting,
            frames: Vec::new(),
            frame: 0,
            scopes: Vec::new(),
            children: HashMap::new(),
            paths: HashMap::new(),
            expanded: ["Locals", "Members"].into_iter().map(String::from).collect(),
            pending: HashMap::new(),
            watches,
            watch_values: HashMap::new(),
            watch_input: String::new(),
        }
    }

    pub fn paused(&self) -> bool {
        matches!(self.state, State::Paused { .. })
    }

    /// The frame shown: its file and one-based line.
    pub fn location(&self) -> Option<(&Path, usize)> {
        let frame = self.frames.get(self.frame)?;
        Some((frame.path.as_deref()?, frame.line))
    }

    fn send(&mut self, request: Request, sent: std::io::Result<i64>) {
        if let Ok(seq) = sent {
            self.pending.insert(seq, request);
        }
    }

    /// Connected: start the game. Breakpoints follow on `initialized`.
    pub fn on_connected(&mut self, client: DapClient, project: &Path) {
        let _ = client.initialize();
        let _ = client.launch(project, self.scene.as_deref());
        self.client = Some(client);
    }

    pub fn on_message(&mut self, message: DapMessage) -> Vec<Effect> {
        match message {
            DapMessage::Event { event, body } => self.on_event(&event, &body),
            DapMessage::Response { request_seq, success, body, message, .. } => {
                let Some(request) = self.pending.remove(&request_seq) else {
                    return match (success, message) {
                        (false, Some(message)) => vec![Effect::Output(format!("Debugger: {message}"), true)],
                        _ => Vec::new(),
                    };
                };
                self.on_response(request, success, &body)
            }
        }
    }

    fn on_event(&mut self, event: &str, body: &Value) -> Vec<Effect> {
        match event {
            "initialized" => vec![Effect::Configure],
            "process" => {
                self.state = State::Running;
                Vec::new()
            }
            "output" => {
                let output = body["output"].as_str().unwrap_or_default().replace("\r\n", "\n");
                let error = body["category"].as_str() == Some("stderr");
                vec![Effect::Output(output, error)]
            }
            "stopped" => {
                self.state = State::Paused {
                    reason: body["description"].as_str().or(body["reason"].as_str()).unwrap_or("Paused").to_string(),
                    text: body["text"].as_str().map(str::to_string),
                };
                self.clear_frames();
                let sent = self.client.as_ref().map(DapClient::stack_trace);
                if let Some(sent) = sent {
                    self.send(Request::StackTrace, sent);
                }
                Vec::new()
            }
            "continued" => {
                self.state = State::Running;
                self.clear_frames();
                Vec::new()
            }
            "terminated" | "exited" => vec![Effect::Ended(None)],
            _ => Vec::new(),
        }
    }

    fn on_response(&mut self, request: Request, success: bool, body: &Value) -> Vec<Effect> {
        match request {
            Request::StackTrace => {
                self.frames = dap::stack_frames(body);
                self.select_frame(0)
            }
            Request::Scopes => {
                self.scopes = dap::scopes(body);
                self.children.clear();
                self.paths.clear();
                let scopes: Vec<(i64, String)> = self.scopes.iter().map(|s| (s.reference, s.name.clone())).collect();
                for (reference, name) in scopes {
                    self.paths.insert(reference, name.clone());
                    if self.expanded.contains(&name) {
                        self.request_variables(reference, 0);
                    }
                }
                Vec::new()
            }
            Request::Watch(expression) => {
                let value = match (success, body["result"].as_str()) {
                    (true, Some(result)) => (result.to_string(), body["variablesReference"].as_i64().unwrap_or_default()),
                    _ => ("(can't evaluate)".to_string(), 0),
                };
                if value.1 != 0 {
                    self.paths.insert(value.1, format!("Watch/{expression}"));
                    if self.expanded.contains(&format!("Watch/{expression}")) {
                        self.request_variables(value.1, 0);
                    }
                }
                self.watch_values.insert(expression, value);
                Vec::new()
            }
            Request::Hover(pos, expression) => match (success, body["result"].as_str()) {
                (true, Some(result)) => vec![Effect::HoverValue(pos, expression, result.to_string())],
                _ => Vec::new(),
            },
            Request::Variables { reference, attempt } if !success => {
                if attempt + 1 < MAX_ATTEMPTS && self.paused() {
                    vec![Effect::Retry(reference, attempt + 1)]
                } else {
                    Vec::new()
                }
            }
            Request::Variables { reference, .. } => {
                let variables = dap::variables(body);
                let parent = self.paths.get(&reference).cloned().unwrap_or_default();
                let mut open = Vec::new();
                for variable in &variables {
                    if variable.reference != 0 {
                        let path = format!("{parent}/{}", variable.name);
                        if self.expanded.contains(&path) {
                            open.push(variable.reference);
                        }
                        self.paths.insert(variable.reference, path);
                    }
                }
                self.children.insert(reference, variables);
                for child in open {
                    self.request_variables(child, 0);
                }
                Vec::new()
            }
        }
    }

    pub fn request_variables(&mut self, reference: i64, attempt: u32) {
        let sent = self.client.as_ref().map(|c| c.variables(reference));
        if let Some(sent) = sent {
            self.send(Request::Variables { reference, attempt }, sent);
        }
    }

    fn clear_frames(&mut self) {
        self.watch_values.clear();
        self.frames.clear();
        self.frame = 0;
        self.scopes.clear();
        self.children.clear();
        self.paths.clear();
    }

    /// Shows frame `index`: its line, and its variables.
    pub fn select_frame(&mut self, index: usize) -> Vec<Effect> {
        let Some(frame) = self.frames.get(index) else {
            return Vec::new();
        };
        self.frame = index;
        let id = frame.id;
        let jump = frame.path.clone().map(|path| Effect::Jump(path, frame.line.saturating_sub(1)));
        self.scopes.clear();
        self.children.clear();
        let sent = self.client.as_ref().map(|c| c.scopes(id));
        if let Some(sent) = sent {
            self.send(Request::Scopes, sent);
        }
        self.watch_values.clear();
        for expression in self.watches.clone() {
            self.evaluate_watch(expression);
        }
        jump.into_iter().collect()
    }

    /// Evaluates a watch in the frame shown (when paused).
    pub fn evaluate_watch(&mut self, expression: String) {
        let Some(frame) = self.frames.get(self.frame).map(|f| f.id) else {
            return;
        };
        let sent = self.client.as_ref().map(|c| c.evaluate(&expression, frame, "watch"));
        if let Some(sent) = sent {
            self.send(Request::Watch(expression), sent);
        }
    }

    /// Asks for the value of `expression`, hovered at `pos` (when paused).
    pub fn hover(&mut self, pos: Pos, expression: String) {
        let Some(frame) = self.frames.get(self.frame).map(|f| f.id) else {
            return;
        };
        let sent = self.client.as_ref().map(|c| c.evaluate(&expression, frame, "hover"));
        if let Some(sent) = sent {
            self.send(Request::Hover(pos, expression), sent);
        }
    }

    pub fn toggle(&mut self, path: String, reference: i64) {
        if self.expanded.remove(&path) {
            return;
        }
        self.expanded.insert(path);
        if !self.children.contains_key(&reference) {
            self.request_variables(reference, 0);
        }
    }

    /// Continue, step or pause.
    pub fn step(&mut self, command: &str) {
        if let Some(client) = &self.client {
            let _ = client.step(command);
        }
    }

    pub fn stop(&mut self) {
        if let Some(client) = &self.client {
            let _ = client.disconnect();
        }
    }
}

/// What hovering `column` of `line` should evaluate while paused: the name
/// there with the `a.b.` chain before it (`data.hp`, `self.speed`). Never
/// a call: evaluating `f()` would run it in the game.
pub fn hover_expression(line: &str, column: usize) -> Option<String> {
    let chars: Vec<char> = line.chars().collect();
    let is_name = |c: char| c.is_alphanumeric() || c == '_';
    if !chars.get(column).copied().is_some_and(is_name) {
        return None;
    }
    let mut end = column;
    while end < chars.len() && is_name(chars[end]) {
        end += 1;
    }
    // Calling it (`name(`): not a value.
    if chars[end..].iter().find(|c| !c.is_whitespace()) == Some(&'(') {
        return None;
    }
    let mut start = column;
    loop {
        while start > 0 && is_name(chars[start - 1]) {
            start -= 1;
        }
        // `a.b`: take `a.` too. A member of a call or index (`f().b`) can't
        // be evaluated without the call: nothing (`b` alone would be self's).
        if start >= 1 && chars[start - 1] == '.' {
            if start >= 2 && is_name(chars[start - 2]) {
                start -= 1;
                continue;
            }
            return None;
        }
        break;
    }
    let expression: String = chars[start..end].iter().collect();
    let first = expression.chars().next()?;
    (!first.is_ascii_digit()).then_some(expression)
}

/// Sends every file's breakpoints (one-based lines), including files whose
/// last one was removed, so Godot forgets them.
pub fn send_breakpoints(client: &DapClient, breakpoints: &BTreeMap<PathBuf, BTreeSet<usize>>) {
    for (path, lines) in breakpoints {
        let lines: Vec<usize> = lines.iter().map(|l| l + 1).collect();
        let _ = client.set_breakpoints(path, &lines);
    }
}

pub struct Look {
    pub text: Color,
    pub muted: Color,
    pub accent: Color,
    pub error: Color,
    pub highlight: Color,
}

impl Session {
    /// The DEBUG panel: state, call stack on the left, variables on the right.
    pub fn view<'a>(&'a self, look: &Look, display: impl Fn(&Path) -> String) -> Element<'a, DebugMessage> {
        let status: Element<'a, DebugMessage> = match &self.state {
            State::Connecting => text("Starting the game...").size(12).color(look.muted).into(),
            State::Running => text("Running").size(12).color(look.muted).into(),
            State::Paused { reason, text: None } => text(format!("Paused: {reason}")).size(12).color(look.accent).into(),
            State::Paused { reason, text: Some(message) } => {
                column![text(format!("Paused: {reason}")).size(12).color(look.error), text(message.clone()).size(12).color(look.error)].into()
            }
        };

        let mut frames: Vec<Element<'a, DebugMessage>> = vec![text("CALL STACK").size(11).color(look.muted).into()];
        for (index, frame) in self.frames.iter().enumerate() {
            let place = frame.path.as_deref().map(|p| format!("{}:{}", display(p), frame.line)).unwrap_or_default();
            let content = row![text(frame.name.clone()).size(12).font(Font::MONOSPACE), text(place).size(11).color(look.muted)].spacing(8);
            let selected = index == self.frame;
            let highlight = look.highlight;
            frames.push(
                button(content)
                    .on_press(DebugMessage::SelectFrame(index))
                    .width(Length::Fill)
                    .padding([1, 6])
                    .style(move |theme: &iced::Theme, status| {
                        let base = button::text(theme, status);
                        if selected {
                            button::Style { background: Some(Background::Color(highlight)), ..base }
                        } else {
                            base
                        }
                    })
                    .into(),
            );
        }
        if self.frames.is_empty() && self.paused() {
            frames.push(text("Loading...").size(12).color(look.muted).into());
        }

        let mut variables: Vec<Element<'a, DebugMessage>> = vec![text("WATCH").size(11).color(look.muted).into()];
        for (index, expression) in self.watches.iter().enumerate() {
            let (value, reference) = match self.watch_values.get(expression) {
                Some((value, reference)) => (Some(value.as_str()), *reference),
                None => (self.paused().then_some("..."), 0),
            };
            let path = format!("Watch/{expression}");
            self.push_node(&mut variables, path, expression, value, reference, 0, look, Some(DebugMessage::RemoveWatch(index)));
        }
        variables.push(
            text_input("Add a watch expression (e.g. velocity.length())", &self.watch_input)
                .on_input(DebugMessage::WatchInput)
                .on_submit(DebugMessage::AddWatch)
                .size(12)
                .padding([2, 6])
                .into(),
        );
        variables.push(text("VARIABLES").size(11).color(look.muted).into());
        for scope in &self.scopes {
            self.push_node(&mut variables, scope.name.clone(), &scope.name, None, scope.reference, 0, look, None);
        }
        if !self.paused() {
            variables.push(text("Shown while paused (breakpoint, step, error)").size(12).color(look.muted).into());
        }

        let stack = scrollable(Column::with_children(frames).spacing(1).padding([4, 8])).height(Length::Fill).width(Length::FillPortion(2));
        let vars = scrollable(Column::with_children(variables).spacing(1).padding([4, 8])).height(Length::Fill).width(Length::FillPortion(3));
        column![container(status).padding([2, 8]), row![stack, vars].height(Length::Fill)].into()
    }

    /// A scope or variable row and, when expanded, its children.
    #[allow(clippy::too_many_arguments)]
    fn push_node<'a>(
        &'a self,
        rows: &mut Vec<Element<'a, DebugMessage>>,
        path: String,
        name: &'a str,
        value: Option<&'a str>,
        reference: i64,
        depth: usize,
        look: &Look,
        remove: Option<DebugMessage>,
    ) {
        let expandable = reference != 0;
        let open = expandable && self.expanded.contains(&path);
        let arrow = match (expandable, open) {
            (false, _) => " ",
            (true, false) => "▸",
            (true, true) => "▾",
        };
        let mut content = row![text(arrow).size(12).width(12), text(name).size(12).font(Font::MONOSPACE).color(look.text)]
            .spacing(4)
            .align_y(Alignment::Center);
        if let Some(value) = value {
            content = content.push(text("=").size(12).color(look.muted)).push(text(value).size(12).font(Font::MONOSPACE).color(look.accent));
        }
        let padding = iced::Padding { left: 4.0 + 14.0 * depth as f32, right: 4.0, top: 0.0, bottom: 0.0 };
        let row_button = button(content).width(Length::Fill).padding(padding).style(button::text);
        let row_button: Element<'a, DebugMessage> =
            if expandable { row_button.on_press(DebugMessage::Toggle(path.clone(), reference)).into() } else { row_button.into() };
        rows.push(match remove {
            Some(remove) => {
                let close = button(text("×").size(12)).on_press(remove).style(button::text).padding([0, 6]);
                row![row_button, close].align_y(Alignment::Center).into()
            }
            None => row_button,
        });
        if !open {
            return;
        }
        match self.children.get(&reference) {
            Some(children) if children.is_empty() => {
                rows.push(container(text("(empty)").size(11).color(look.muted)).padding(iced::Padding { left: 30.0 + 14.0 * depth as f32, ..Default::default() }).into());
            }
            Some(children) => {
                for child in children {
                    let child_path = format!("{path}/{}", child.name);
                    self.push_node(rows, child_path, &child.name, Some(&child.value), child.reference, depth + 1, look, None);
                }
            }
            None => {
                rows.push(container(text("...").size(11).color(look.muted)).padding(iced::Padding { left: 30.0 + 14.0 * depth as f32, ..Default::default() }).into());
            }
        }
    }
}

/// The DEBUG panel's buttons: continue / pause, the steps, stop.
pub fn controls<'a>(session: &Session, ui: &theme::UiColors, key: impl Fn(&str) -> String) -> Vec<Element<'a, DebugMessage>> {
    let paused = session.paused();
    let action = |label: &'static str, message: Option<DebugMessage>, tip: String| -> Element<'a, DebugMessage> {
        let b = button(text(label).size(12)).style(button::text).padding([0, 6]);
        let b = match message {
            Some(message) => b.on_press(message),
            None => b,
        };
        style::tip(b, tip, Position::Top, ui)
    };
    let with_key = |name: &str, id: &str| match key(id) {
        k if k.is_empty() => name.to_string(),
        k => format!("{name} ({k})"),
    };
    let running = session.state == State::Running;
    vec![
        if paused {
            action("continue", Some(DebugMessage::Step("continue")), "Continue (F5)".to_string())
        } else {
            action("pause", running.then_some(DebugMessage::Step("pause")), "Pause the game".to_string())
        },
        action("over", paused.then_some(DebugMessage::Step("next")), with_key("Step over", "debug.stepOver")),
        action("into", paused.then_some(DebugMessage::Step("stepIn")), with_key("Step into", "debug.stepInto")),
        action("out", paused.then_some(DebugMessage::Step("stepOut")), with_key("Step out", "debug.stepOut")),
        action("stop", Some(DebugMessage::Stop), with_key("Stop debugging", "run.stop")),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn response(seq: i64, success: bool, body: Value) -> DapMessage {
        DapMessage::Response { request_seq: seq, command: String::new(), success, body, message: None }
    }

    #[test]
    fn hovering_evaluates_names_and_member_chains_never_calls() {
        let at = |text: &str| {
            let column = text.find('|').unwrap();
            hover_expression(&text.replace('|', ""), column)
        };
        assert_eq!(at("\tcounter += |i").as_deref(), Some("i"));
        assert_eq!(at("\tprint(data.h|p)").as_deref(), Some("data.hp"));
        assert_eq!(at("\tself.spe|ed = 2").as_deref(), Some("self.speed"));
        assert_eq!(at("\tse|lf.speed = 2").as_deref(), Some("self"));
        assert_eq!(at("\tvar v = position.normalized|()"), None);
        assert_eq!(at("\tget_node(\"x\").vis|ible"), None);
        assert_eq!(at("\titems[0].na|me"), None);
        assert_eq!(at("\tx = 4|2"), None);
        assert_eq!(at("\tx = |+ 1"), None);
    }

    #[test]
    fn a_pause_loads_the_stack_then_the_expanded_variables() {
        let mut session = Session::new(1, 0, None, Vec::new());
        // Without a client nothing is sent: fake the requests' bookkeeping.
        let stopped = DapMessage::Event { event: "stopped".into(), body: json!({ "description": "Breakpoint" }) };
        assert!(session.on_message(stopped).is_empty());
        assert_eq!(session.state, State::Paused { reason: "Breakpoint".into(), text: None });

        session.pending.insert(1, Request::StackTrace);
        let frames = json!({ "stackFrames": [{ "id": 0, "line": 9, "name": "_ready", "source": { "path": "C:/game/main.gd" } }] });
        let effects = session.on_message(response(1, true, frames));
        assert!(matches!(&effects[..], [Effect::Jump(_, 8)]));

        session.pending.insert(2, Request::Scopes);
        session.on_message(response(2, true, json!({ "scopes": [
            { "name": "Locals", "variablesReference": 1 }, { "name": "Globals", "variablesReference": 3 }
        ] })));
        assert_eq!(session.paths.get(&1).map(String::as_str), Some("Locals"));

        // Not ready yet: retried, then loaded.
        session.pending.insert(3, Request::Variables { reference: 1, attempt: 0 });
        assert_eq!(session.on_message(response(3, false, json!({}))), [Effect::Retry(1, 1)]);
        session.pending.insert(4, Request::Variables { reference: 1, attempt: 1 });
        session.on_message(response(4, true, json!({ "variables": [
            { "name": "self", "value": "<Object>", "variablesReference": 4 },
            { "name": "@pos", "value": "0", "variablesReference": 0 }
        ] })));
        assert_eq!(session.children[&1].len(), 1);
        assert_eq!(session.paths.get(&4).map(String::as_str), Some("Locals/self"));

        // Watches and hovers are evaluated in the paused frame.
        session.pending.insert(5, Request::Watch("counter".into()));
        session.on_message(response(5, true, json!({ "result": "3", "variablesReference": 0 })));
        assert_eq!(session.watch_values.get("counter"), Some(&("3".to_string(), 0)));
        let pos = Pos { line: 8, column: 2 };
        session.pending.insert(6, Request::Hover(pos, "data.hp".into()));
        let hover = session.on_message(response(6, true, json!({ "result": "10", "variablesReference": 0 })));
        assert_eq!(hover, [Effect::HoverValue(pos, "data.hp".into(), "10".into())]);

        // Expanding is remembered by path; continuing clears the frame.
        session.toggle("Locals/self".into(), 4);
        assert!(session.expanded.contains("Locals/self"));
        session.on_message(DapMessage::Event { event: "continued".into(), body: json!({}) });
        assert!(session.frames.is_empty() && session.children.is_empty() && session.expanded.contains("Locals/self"));
        assert!(session.watch_values.is_empty());
        let ended = session.on_message(DapMessage::Event { event: "terminated".into(), body: Value::Null });
        assert_eq!(ended, [Effect::Ended(None)]);
    }
}
