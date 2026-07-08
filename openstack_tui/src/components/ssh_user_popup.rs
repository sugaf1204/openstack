// Licensed under the Apache License, Version 2.0 (the "License");
// you may not use this file except in compliance with the License.
// You may obtain a copy of the License at
//
//     http://www.apache.org/licenses/LICENSE-2.0
//
// Unless required by applicable law or agreed to in writing, software
// distributed under the License is distributed on an "AS IS" BASIS,
// WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
// See the License for the specific language governing permissions and
// limitations under the License.
//
// SPDX-License-Identifier: Apache-2.0

use std::env;

use crossterm::event::{KeyCode, KeyEvent};
use eyre::Result;
use ratatui::{layout::Rect, prelude::*, widgets::*};

use crate::{
    action::Action, components::Component, config::Config, error::TuiError, mode::Mode,
    utils::centered_rect_fixed, widgets::button::Button,
};

pub struct SshUserPopup {
    config: Config,
    input: String,
    host: Option<String>,
    connect_button: Button<'static>,
}

impl SshUserPopup {
    pub fn new() -> Self {
        Self {
            config: Config::default(),
            input: String::new(),
            host: None,
            connect_button: Button::new("Connect"),
        }
    }

    fn reset(&mut self) {
        self.input.clear();
        self.host = None;
    }
}

impl Default for SshUserPopup {
    fn default() -> Self {
        Self::new()
    }
}

impl Component for SshUserPopup {
    fn register_config_handler(&mut self, config: Config) -> Result<(), TuiError> {
        self.config = config;
        Ok(())
    }

    fn handle_key_events(&mut self, key: KeyEvent) -> Result<Option<Action>, TuiError> {
        match key.code {
            KeyCode::Enter => {
                let Some(host) = self.host.clone() else {
                    self.reset();
                    return Ok(None);
                };
                let destination = ssh_destination(&self.input, &host);
                self.reset();
                Ok(Some(Action::RunTerminalCommand {
                    program: String::from("ssh"),
                    args: vec![destination],
                }))
            }
            KeyCode::Esc => {
                self.reset();
                Ok(None)
            }
            KeyCode::Backspace | KeyCode::Delete => {
                self.input.pop();
                Ok(None)
            }
            KeyCode::Char(ch) => {
                self.input.push(ch);
                Ok(None)
            }
            _ => Ok(None),
        }
    }

    fn update(&mut self, action: Action, _current_mode: Mode) -> Result<Option<Action>, TuiError> {
        if let Action::PromptSshUser { host } = action {
            self.host = Some(host);
            self.input = runtime_ssh_user();
        }
        Ok(None)
    }

    fn draw(&mut self, frame: &mut Frame<'_>, _area: Rect) -> Result<(), TuiError> {
        let ar = centered_rect_fixed(50, 10, frame.area());
        let popup_block = Block::default()
            .title_top(Line::from("SSH connection").light_yellow().centered())
            .borders(Borders::ALL)
            .border_type(BorderType::Thick)
            .padding(Padding::uniform(1))
            .bg(self.config.styles.popup_bg)
            .border_style(Style::default().fg(self.config.styles.popup_border_confirm_fg));

        let input_block = Block::default()
            .title("User")
            .borders(Borders::ALL)
            .border_style(Style::default().fg(self.config.styles.fg));

        let areas = Layout::default()
            .constraints(
                [
                    Constraint::Length(1),
                    Constraint::Length(3),
                    Constraint::Percentage(100),
                    Constraint::Length(1),
                ]
                .as_ref(),
            )
            .split(popup_block.inner(ar));

        let host = Paragraph::new(format!(
            "Host: {}",
            self.host.as_deref().unwrap_or_default()
        ));
        let input = Paragraph::new(self.input.clone()).block(input_block);

        frame.render_widget(Clear, ar);
        frame.render_widget(popup_block, ar);
        frame.render_widget(host, areas[0]);
        frame.render_widget(input, areas[1]);
        frame.render_widget(&self.connect_button, areas[3]);

        Ok(())
    }
}

fn runtime_ssh_user() -> String {
    ["USER", "LOGNAME", "USERNAME"]
        .iter()
        .find_map(|key| env::var(key).ok().filter(|value| !value.trim().is_empty()))
        .unwrap_or_default()
}

fn ssh_destination(user: &str, host: &str) -> String {
    let user = user.trim();
    if user.is_empty() {
        host.to_string()
    } else {
        format!("{user}@{host}")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn key(code: KeyCode) -> KeyEvent {
        KeyEvent::new(code, crossterm::event::KeyModifiers::empty())
    }

    #[test]
    fn ssh_destination_uses_user_when_present() {
        assert_eq!(ssh_destination("ubuntu", "10.0.0.5"), "ubuntu@10.0.0.5");
    }

    #[test]
    fn ssh_destination_trims_user() {
        assert_eq!(ssh_destination(" ubuntu ", "10.0.0.5"), "ubuntu@10.0.0.5");
    }

    #[test]
    fn ssh_destination_uses_host_without_user() {
        assert_eq!(ssh_destination("", "10.0.0.5"), "10.0.0.5");
    }

    #[test]
    fn prompt_sets_runtime_user_and_host() {
        let mut popup = SshUserPopup::new();
        popup
            .update(
                Action::PromptSshUser {
                    host: String::from("10.0.0.5"),
                },
                Mode::ComputeServers,
            )
            .unwrap();

        assert_eq!(popup.host.as_deref(), Some("10.0.0.5"));
        assert_eq!(popup.input, runtime_ssh_user());
    }

    #[test]
    fn enter_returns_ssh_command_with_selected_user() {
        let mut popup = SshUserPopup::new();
        popup.host = Some(String::from("10.0.0.5"));
        popup.input = String::from("ubuntu");

        let action = popup.handle_key_events(key(KeyCode::Enter)).unwrap();

        assert_eq!(
            action,
            Some(Action::RunTerminalCommand {
                program: String::from("ssh"),
                args: vec![String::from("ubuntu@10.0.0.5")]
            })
        );
        assert_eq!(popup.host, None);
        assert_eq!(popup.input, "");
    }

    #[test]
    fn esc_resets_prompt() {
        let mut popup = SshUserPopup::new();
        popup.host = Some(String::from("10.0.0.5"));
        popup.input = String::from("ubuntu");

        let action = popup.handle_key_events(key(KeyCode::Esc)).unwrap();

        assert_eq!(action, None);
        assert_eq!(popup.host, None);
        assert_eq!(popup.input, "");
    }
}
