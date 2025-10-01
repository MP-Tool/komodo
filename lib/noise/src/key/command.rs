use anyhow::Context;
use colored::Colorize;
use komodo_client::entities::config::{
  KeyCommand, KeyOutputFormat, KeyPair,
};

use super::SpkiPublicKey;

pub async fn handle(command: &KeyCommand) -> anyhow::Result<()> {
  match command {
    KeyCommand::Generate { format } => {
      let keys = super::EncodedKeyPair::generate()
        .context("Failed to generate key pair")?;
      match format {
        KeyOutputFormat::Standard => {
          println!("\nPrivate Key: {}", keys.private.red().bold());
          println!("Public  Key: {}", keys.public.bold());
        }
        KeyOutputFormat::Json => {
          print_json(&keys.private, &keys.public)?
        }
        KeyOutputFormat::JsonPretty => {
          print_json_pretty(&keys.private, &keys.public)?
        }
      }

      Ok(())
    }
    KeyCommand::Compute {
      private_key,
      format,
    } => {
      let public_key = SpkiPublicKey::from_private_key(private_key)
        .context("Failed to compute public key")?
        .into_inner();
      match format {
        KeyOutputFormat::Standard => {
          println!("\nPublic Key: {}", public_key.bold());
        }
        KeyOutputFormat::Json => {
          print_json(private_key, &public_key)?
        }
        KeyOutputFormat::JsonPretty => {
          print_json_pretty(private_key, &public_key)?
        }
      }
      Ok(())
    }
  }
}

fn print_json(
  private_key: &str,
  public_key: &str,
) -> anyhow::Result<()> {
  let json = serde_json::to_string(&KeyPair {
    private_key,
    public_key,
  })
  .context("Failed to serialize JSON")?;
  println!("{json}");
  Ok(())
}

fn print_json_pretty(
  private_key: &str,
  public_key: &str,
) -> anyhow::Result<()> {
  let json = serde_json::to_string_pretty(&KeyPair {
    private_key,
    public_key,
  })
  .context("Failed to serialize JSON")?;
  println!("{json}");
  Ok(())
}
