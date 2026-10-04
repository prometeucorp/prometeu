//! Application models and ports shared by desktop and execution hosts.
//! Native effects and transport implementations belong to the composing host.

pub mod accounts;
pub mod actions;
pub mod automation;
pub mod board;
pub mod conversation;
pub mod delegation;
pub mod domain;
pub mod error;
pub mod identities;
pub mod lock;
pub mod process;
pub mod publication;
pub mod selection;
pub mod workspace_lifecycle;
pub mod workspace_tools;
pub mod workspaces;

pub mod session;

pub mod terminal;

pub mod auxiliary;

pub mod command;

pub mod tasks;

pub mod files;

pub mod repository;

pub mod agents;

pub mod workspace_draft;

pub mod git;

pub mod projects;

pub mod tool_resolution;
