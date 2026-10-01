//! Inline forms (`ShowForm`): a few labeled text or password fields in a
//! rounded box that replaces the input box until submitted or cancelled.

use crate::editor::{EditKey, EditOutcome, Editor};
use crate::session::Key;
use crate::style::{Line, Style};
use crate::theme::Theme;
use crate::width::{str_width, truncate};
use serde_json::{json, Map, Value};

pub struct Field {
    pub name: String,
    pub label: String,
    pub password: bool,
    pub placeholder: String,
    pub required: bool,
    pub editor: Editor,
}

pub struct FormState {
    pub id: String,
    pub title: String,
    pub allow_cancel: bool,
    pub fields: Vec<Field>,
    pub focused: usize,
    /// A message under the fields, e.g. a required field left empty.
    pub error: Option<String>,
}

/// What a key press did to the form.
pub enum FormOutcome {
    Pending,
    Submitted(Map<String, Value>),
    Cancelled,
}

impl FormState {
    pub fn from_payload(p: &Value) -> Self {
        let s = |v: &Value, k: &str| v.get(k).and_then(Value::as_str).unwrap_or("").to_string();
        let fields = p
            .get("fields")
            .and_then(Value::as_array)
            .map(|a| {
                a.iter()
                    .map(|f| {
                        let mut editor = Editor::new();
                        if let Some(d) = f.get("default").and_then(Value::as_str) {
                            editor.set_text(d);
                        }
                        Field {
                            name: s(f, "name"),
                            label: s(f, "label"),
                            password: f.get("kind").and_then(Value::as_str) == Some("password"),
                            placeholder: s(f, "placeholder"),
                            required: f.get("required").and_then(Value::as_bool).unwrap_or(false),
                            editor,
                        }
                    })
                    .collect()
            })
            .unwrap_or_default();
        FormState {
            id: s(p, "id"),
            title: s(p, "title"),
            allow_cancel: p.get("allow_cancel").and_then(Value::as_bool).unwrap_or(true),
            fields,
            focused: 0,
            error: None,
        }
    }

    pub fn key(&mut self, key: Key) -> FormOutcome {
        let n = self.fields.len().max(1);
        match key {
            Key::Esc | Key::Ctrl('c') if self.allow_cancel => return FormOutcome::Cancelled,
            Key::Tab | Key::Down => self.focused = (self.focused + 1) % n,
            Key::BackTab | Key::Up => self.focused = (self.focused + n - 1) % n,
            Key::Enter { .. } => {
                if self.focused + 1 < self.fields.len() {
                    self.focused += 1;
                } else {
                    return self.submit();
                }
            }
            other => {
                let edit = match other {
                    Key::Text(t) => EditKey::Text(t.replace('\n', " ")),
                    Key::Paste(t) => EditKey::Text(t.replace(['\n', '\r'], " ")),
                    Key::Backspace { word } => EditKey::Backspace { word },
                    Key::Ctrl('w') => EditKey::Backspace { word: true },
                    Key::Delete { word } => EditKey::Delete { word },
                    Key::Left { word } => EditKey::Left { word },
                    Key::Right { word } => EditKey::Right { word },
                    Key::Home | Key::Ctrl('a') => EditKey::LineStart,
                    Key::End | Key::Ctrl('e') => EditKey::LineEnd,
                    Key::Ctrl('u') => EditKey::KillToStart,
                    Key::Ctrl('k') => EditKey::KillToEnd,
                    Key::Ctrl('y') => EditKey::Yank,
                    Key::Ctrl('z') => EditKey::Undo,
                    _ => return FormOutcome::Pending,
                };
                if let Some(f) = self.fields.get_mut(self.focused) {
                    if let EditOutcome::Changed = f.editor.handle(edit, 1000) {
                        self.error = None;
                    }
                }
            }
        }
        FormOutcome::Pending
    }

    fn submit(&mut self) -> FormOutcome {
        if let Some(i) = self.fields.iter().position(|f| f.required && f.editor.text().trim().is_empty()) {
            self.focused = i;
            self.error = Some(format!("{} is required", self.fields[i].label));
            return FormOutcome::Pending;
        }
        let mut values = Map::new();
        for f in &self.fields {
            values.insert(f.name.clone(), json!(f.editor.text()));
        }
        FormOutcome::Submitted(values)
    }

    /// The form box and the cursor cell (row, column) within it.
    pub fn render(&self, theme: &Theme, width: usize) -> (Vec<Line>, (usize, usize)) {
        let inner = width.saturating_sub(4).max(8);
        let border = theme.accent();
        let mut content: Vec<Line> = Vec::new();
        if !self.title.is_empty() {
            content.push(Line::styled(truncate(&self.title, inner), theme.strong()));
            content.push(Line::new());
        }
        let mut cursor = (0, 0);
        for (i, f) in self.fields.iter().enumerate() {
            let focused = i == self.focused;
            let mut label = Line::styled(f.label.clone(), if focused { theme.accent().bold() } else { theme.dim() });
            if f.required {
                label.push(" *", theme.dim());
            }
            content.push(label);
            let text = f.editor.text();
            let shown = if f.password { "•".repeat(text.chars().count()) } else { text.to_string() };
            let field_w = inner.saturating_sub(2);
            // Keep the cursor visible in a long value by showing its tail.
            let cur_chars = if f.password { text[..f.editor.cursor()].chars().count() } else { 0 };
            let before = if f.password { "•".repeat(cur_chars) } else { text[..f.editor.cursor()].to_string() };
            let skip = str_width(&before).saturating_sub(field_w.saturating_sub(1));
            let visible: String = shown.chars().skip(skip).collect();
            let mut line = Line::styled(if focused { "› " } else { "  " }, theme.accent());
            if text.is_empty() && !f.placeholder.is_empty() {
                line.push(truncate(&f.placeholder, field_w), theme.dim());
            } else {
                line.push(truncate(&visible, field_w), Style::new());
            }
            if focused {
                cursor = (content.len(), 2 + str_width(&before) - skip);
            }
            content.push(line);
            content.push(Line::new());
        }
        if let Some(err) = &self.error {
            content.push(Line::styled(truncate(err, inner), theme.err()));
        }
        let mut out = vec![Line::styled(format!("╭{}╮", "─".repeat(width.saturating_sub(2))), border)];
        for c in content {
            let pad = inner.saturating_sub(c.width());
            let mut l = Line::styled("│ ", border);
            l.spans.extend(c.spans);
            l.push(" ".repeat(pad), Style::new());
            l.push(" │", border);
            out.push(l);
        }
        out.push(Line::styled(format!("╰{}╯", "─".repeat(width.saturating_sub(2))), border));
        let hint = if self.allow_cancel { "tab to move · enter to continue · esc to cancel" } else { "tab to move · enter to continue" };
        out.push(Line::styled(format!("  {hint}"), theme.dim()));
        // +1 for the top border, +2 for "│ ".
        (out, (cursor.0 + 1, cursor.1 + 2))
    }
}
