//! Integration tests of the viewer context: schedule and clocks, tasks, events, state, checks,
//! the periodic schedule, commands, event presets and shutdown. None touches the network except
//! a refused local connection.

mod check;
mod clock;
mod commands;
mod events;
mod network;
mod presets;
mod schedule;
mod shutdown;
mod state;
mod support;
mod tasks;
