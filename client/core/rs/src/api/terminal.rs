use serde::{Deserialize, Serialize};
use typeshare::typeshare;

use crate::api::write::TerminalRecreateMode;

/// Query to connect to a terminal (interactive shell over websocket) on the given server.
#[typeshare]
#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct ConnectTerminalQuery {
  /// Server Id or name
  pub server: String,
  /// Each periphery can keep multiple terminals open.
  /// If a terminals with the specified name does not exist,
  /// the call will fail.
  /// Create a terminal using [CreateTerminal][super::write::server::CreateTerminal]
  pub terminal: String,
}

/// Execute a terminal command on the given server.
#[typeshare]
#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct ExecuteTerminalBody {
  /// Server Id or name
  pub server: String,
  /// The name of the terminal on the server to use to execute.
  pub terminal: String,
  /// The command to execute.
  pub command: String,
}

/// Query to connect to a container exec session (interactive shell over websocket) on the given server.
#[typeshare]
#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct ConnectContainerExecQuery {
  /// Server Id or name
  pub server: String,
  /// The container name
  pub container: String,
  /// The shell to use (eg. `sh` or `bash`)
  pub shell: String,
  /// Specify the recreate behavior.
  /// Default is 'DifferentCommand'
  #[serde(default = "default_container_recreate_mode")]
  pub recreate: TerminalRecreateMode,
}

/// Query to connect to a container attach session (interactive shell over websocket) on the given server.
#[typeshare]
#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct ConnectContainerAttachQuery {
  /// Server Id or name
  pub server: String,
  /// The container name
  pub container: String,
  /// Specify the recreate behavior.
  /// Default is 'DifferentCommand'
  #[serde(default = "default_container_recreate_mode")]
  pub recreate: TerminalRecreateMode,
}

/// Execute a command in the given containers shell.
#[typeshare]
#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct ExecuteContainerExecBody {
  /// Server Id or name
  pub server: String,
  /// The container name
  pub container: String,
  /// The shell to use (eg. `sh` or `bash`)
  pub shell: String,
  /// The command to execute.
  pub command: String,
  /// Specify the recreate behavior.
  /// Default is 'DifferentCommand'
  #[serde(default = "default_container_recreate_mode")]
  pub recreate: TerminalRecreateMode,
}

/// Query to connect to a container exec session (interactive shell over websocket) on the given Deployment.
/// This call will use access to the Deployment Terminal to permission the call.
#[typeshare]
#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct ConnectDeploymentExecQuery {
  /// Deployment Id or name
  pub deployment: String,
  /// The shell to use (eg. `sh` or `bash`)
  pub shell: String,
  /// Specify the recreate behavior.
  /// Default is 'DifferentCommand'
  #[serde(default = "default_container_recreate_mode")]
  pub recreate: TerminalRecreateMode,
}

/// Query to connect to a container attach session (interactive shell over websocket) on the given Deployment.
/// This call will use access to the Deployment Terminal to permission the call.
#[typeshare]
#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct ConnectDeploymentAttachQuery {
  /// Deployment Id or name
  pub deployment: String,
  /// Specify the recreate behavior.
  /// Default is 'DifferentCommand'
  #[serde(default = "default_container_recreate_mode")]
  pub recreate: TerminalRecreateMode,
}

/// Execute a command in the given containers shell.
#[typeshare]
#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct ExecuteDeploymentExecBody {
  /// Deployment Id or name
  pub deployment: String,
  /// The shell to use (eg. `sh` or `bash`)
  pub shell: String,
  /// The command to execute.
  pub command: String,
  /// Specify the recreate behavior.
  /// Default is 'DifferentCommand'
  #[serde(default = "default_container_recreate_mode")]
  pub recreate: TerminalRecreateMode,
}

/// Query to connect to a container exec session (interactive shell over websocket) on the given Stack / service.
/// This call will use access to the Stack Terminal to permission the call.
#[typeshare]
#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct ConnectStackExecQuery {
  /// Stack Id or name
  pub stack: String,
  /// The service name to connect to
  pub service: String,
  /// The shell to use (eg. `sh` or `bash`)
  pub shell: String,
  /// Specify the recreate behavior.
  /// Default is 'DifferentCommand'
  #[serde(default = "default_container_recreate_mode")]
  pub recreate: TerminalRecreateMode,
}

/// Query to connect to a container attach session (interactive shell over websocket) on the given Stack / service.
/// This call will use access to the Stack Terminal to permission the call.
#[typeshare]
#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct ConnectStackAttachQuery {
  /// Stack Id or name
  pub stack: String,
  /// The service name to attach to
  pub service: String,
  /// Specify the recreate behavior.
  /// Default is 'DifferentCommand'
  #[serde(default = "default_container_recreate_mode")]
  pub recreate: TerminalRecreateMode,
}

/// Execute a command in the given containers shell.
#[typeshare]
#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct ExecuteStackExecBody {
  /// Stack Id or name
  pub stack: String,
  /// The service name to connect to
  pub service: String,
  /// The shell to use (eg. `sh` or `bash`)
  pub shell: String,
  /// The command to execute.
  pub command: String,
  /// Specify the recreate behavior.
  /// Default is 'DifferentCommand'
  #[serde(default = "default_container_recreate_mode")]
  pub recreate: TerminalRecreateMode,
}

fn default_container_recreate_mode() -> TerminalRecreateMode {
  TerminalRecreateMode::DifferentCommand
}
