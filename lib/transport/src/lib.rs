pub mod auth;
pub mod channel;
pub mod timeout;
pub mod websocket;

/// - Fixes ws addresses:
///   - `server.domain` => `wss://server.domain`
///   - `http://server.domain` => `ws://server.domain`
///   - `https://server.domain` => `wss://server.domain`
pub fn fix_ws_address(address: &str) -> String {
  if address.starts_with("ws://") || address.starts_with("wss://") {
    return address.to_string();
  }
  if address.starts_with("http://") {
    return address.replace("http://", "ws://");
  }
  if address.starts_with("https://") {
    return address.replace("https://", "wss://");
  }
  format!("wss://{address}")
}
