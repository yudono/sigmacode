use sigmacode_core::key_store::{encrypt_key, decrypt_key};

fn main() -> anyhow::Result<()> {
    let args: Vec<String> = std::env::args().collect();

    if args.len() < 3 {
        eprintln!("Usage:");
        eprintln!("  sigmacode-keys encrypt <master_key_hex> <plaintext>");
        eprintln!("  sigmacode-keys decrypt <master_key_hex> <encrypted_json>");
        eprintln!("  sigmacode-keys store <master_key_hex> <server_key> [redis_url]");
        eprintln!("  sigmacode-keys load  <master_key_hex> [redis_url]");
        std::process::exit(1);
    }

    let cmd = args[1].as_str();
    let master_key = args[2].as_str();

    match cmd {
        "encrypt" => {
            let plaintext = args.get(3).ok_or_else(|| anyhow::anyhow!("Missing plaintext"))?;
            let encrypted = encrypt_key(plaintext, master_key)?;
            println!("{}", encrypted);
        }
        "decrypt" => {
            let encrypted = args.get(3).ok_or_else(|| anyhow::anyhow!("Missing encrypted JSON"))?;
            let decrypted = decrypt_key(encrypted, master_key)?;
            println!("{}", decrypted);
        }
        "store" => {
            let server_key = args.get(3).ok_or_else(|| anyhow::anyhow!("Missing server key"))?;
            let redis_url = args.get(4).map(|s| s.as_str()).unwrap_or("redis://127.0.0.1:6379");

            let rt = tokio::runtime::Runtime::new()?;
            rt.block_on(sigmacode_core::key_store::store_key_in_redis(
                server_key, master_key, redis_url,
            ))?;
            println!("Key stored in Redis (encrypted)");
        }
        "load" => {
            let redis_url = args.get(3).map(|s| s.as_str()).unwrap_or("redis://127.0.0.1:6379");

            let rt = tokio::runtime::Runtime::new()?;
            let key = rt.block_on(sigmacode_core::key_store::load_key_from_redis(
                master_key, redis_url,
            ))?;
            println!("{}", key);
        }
        _ => {
            eprintln!("Unknown command: {}", cmd);
            std::process::exit(1);
        }
    }

    Ok(())
}
