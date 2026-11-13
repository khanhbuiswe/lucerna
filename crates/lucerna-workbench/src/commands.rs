//! Named commands (VS Code IDs) that key bindings and UI can run.

use std::collections::BTreeMap;

use gpui::{Context, Window};
use serde_json::Value;

use crate::Workbench;

/// Runs a command against the workbench. `args` comes from the key binding.
pub type CommandHandler = fn(&mut Workbench, Option<&Value>, &mut Window, &mut Context<Workbench>);

#[derive(Clone)]
struct Command {
    title: &'static str,
    handler: CommandHandler,
}

#[derive(Clone, Default)]
pub struct CommandRegistry {
    commands: BTreeMap<&'static str, Command>,
}

impl CommandRegistry {
    /// Registers `id`; registering an existing id replaces it.
    pub fn register(&mut self, id: &'static str, title: &'static str, handler: CommandHandler) {
        self.commands.insert(id, Command { title, handler });
    }

    /// Runs `id`. Returns `false` when no command has that id.
    pub fn run(
        &self,
        id: &str,
        args: Option<&Value>,
        wb: &mut Workbench,
        window: &mut Window,
        cx: &mut Context<Workbench>,
    ) -> bool {
        let Some(command) = self.commands.get(id) else {
            return false;
        };
        (command.handler)(wb, args, window, cx);
        true
    }

    /// The registered ids, in order.
    pub fn ids(&self) -> impl Iterator<Item = &'static str> + '_ {
        self.commands.keys().copied()
    }

    /// `(id, title)` pairs, ordered by id.
    pub fn all(&self) -> impl Iterator<Item = (&str, &str)> {
        self.commands.iter().map(|(id, c)| (*id, c.title))
    }
}
