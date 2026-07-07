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

use crossterm::event::{KeyCode, KeyEvent};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum NavigationKey {
    Up,
    Down,
    Left,
    Right,
}

impl NavigationKey {
    pub(crate) fn from_event(key: &KeyEvent) -> Option<Self> {
        match key.code {
            KeyCode::Up => Some(Self::Up),
            KeyCode::Down => Some(Self::Down),
            KeyCode::Left => Some(Self::Left),
            KeyCode::Right => Some(Self::Right),
            KeyCode::Char('k') if key.modifiers.is_empty() => Some(Self::Up),
            KeyCode::Char('j') if key.modifiers.is_empty() => Some(Self::Down),
            KeyCode::Char('h') if key.modifiers.is_empty() => Some(Self::Left),
            KeyCode::Char('l') if key.modifiers.is_empty() => Some(Self::Right),
            _ => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crossterm::event::KeyModifiers;

    #[test]
    fn maps_arrow_keys() {
        assert_eq!(
            NavigationKey::from_event(&KeyEvent::new(KeyCode::Up, KeyModifiers::empty())),
            Some(NavigationKey::Up)
        );
        assert_eq!(
            NavigationKey::from_event(&KeyEvent::new(KeyCode::Down, KeyModifiers::empty())),
            Some(NavigationKey::Down)
        );
        assert_eq!(
            NavigationKey::from_event(&KeyEvent::new(KeyCode::Left, KeyModifiers::empty())),
            Some(NavigationKey::Left)
        );
        assert_eq!(
            NavigationKey::from_event(&KeyEvent::new(KeyCode::Right, KeyModifiers::empty())),
            Some(NavigationKey::Right)
        );
    }

    #[test]
    fn maps_plain_vim_keys() {
        assert_eq!(
            NavigationKey::from_event(&KeyEvent::new(KeyCode::Char('k'), KeyModifiers::empty())),
            Some(NavigationKey::Up)
        );
        assert_eq!(
            NavigationKey::from_event(&KeyEvent::new(KeyCode::Char('j'), KeyModifiers::empty())),
            Some(NavigationKey::Down)
        );
        assert_eq!(
            NavigationKey::from_event(&KeyEvent::new(KeyCode::Char('h'), KeyModifiers::empty())),
            Some(NavigationKey::Left)
        );
        assert_eq!(
            NavigationKey::from_event(&KeyEvent::new(KeyCode::Char('l'), KeyModifiers::empty())),
            Some(NavigationKey::Right)
        );
    }

    #[test]
    fn ignores_modified_vim_keys() {
        assert_eq!(
            NavigationKey::from_event(&KeyEvent::new(KeyCode::Char('j'), KeyModifiers::CONTROL)),
            None
        );
        assert_eq!(
            NavigationKey::from_event(&KeyEvent::new(KeyCode::Char('l'), KeyModifiers::ALT)),
            None
        );
    }
}
