use base64::Engine;
use clap::{Parser, Subcommand};
use ed25519_dalek::{Signature, Signer, SigningKey, VerifyingKey};
use ed25519_dalek::Verifier;
use rand::rngs::OsRng;
use rand::RngCore;
use sha2::{Digest, Sha256};
use std::fs;
use std::path::Path;

#[derive(Parser)]
#[command(name = "agos-sign", about = "Sign and verify .aapp agent packages")]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    /// Generate a new signing keypair
    GenKey {
        /// Output path for the private key (base64-encoded)
        #[arg(short, long, default_value = "agos-signing-key")]
        output: String,
    },
    /// Sign an .aapp directory
    Sign {
        /// Path to the .aapp directory
        #[arg(short, long)]
        aapp_dir: String,
        /// Path to the base64-encoded private key
        #[arg(short, long)]
        key: String,
    },
    /// Verify an .aapp directory signature
    Verify {
        /// Path to the .aapp directory
        #[arg(short, long)]
        aapp_dir: String,
        /// Path to the base64-encoded public key
        #[arg(short, long)]
        public_key: String,
    },
}

fn main() {
    let cli = Cli::parse();
    match cli.command {
        Commands::GenKey { output } => gen_key(&output),
        Commands::Sign { aapp_dir, key } => sign(&aapp_dir, &key),
        Commands::Verify { aapp_dir, public_key } => verify(&aapp_dir, &public_key),
    }
}

fn gen_key(output_path: &str) {
    let mut rng = OsRng;
    let mut seed = [0u8; 32];
    rng.fill_bytes(&mut seed);
    let signing_key = SigningKey::from_bytes(&seed);
    let verifying_key = signing_key.verifying_key();

    let signing_b64 = base64::engine::general_purpose::STANDARD
        .encode(signing_key.to_bytes());
    let verifying_b64 = base64::engine::general_purpose::STANDARD
        .encode(verifying_key.to_bytes());

    fs::write(format!("{}.key", output_path), &signing_b64)
        .expect("Failed to write private key");
    fs::write(format!("{}.pub", output_path), &verifying_b64)
        .expect("Failed to write public key");

    println!("Generated keypair:");
    println!("  Private key: {}.key", output_path);
    println!("  Public key:  {}.pub", output_path);
}

fn hash_aapp_dir(aapp_dir: &Path) -> Vec<u8> {
    let mut hasher = Sha256::new();
    let mut entries: Vec<_> = fs::read_dir(aapp_dir)
        .expect("Failed to read .aapp directory")
        .filter_map(|e| e.ok())
        .filter(|e| {
            let name = e.file_name();
            let name_str = name.to_string_lossy();
            name_str != "signature.sig" && e.file_type().map(|t| t.is_file()).unwrap_or(false)
        })
        .collect();
    entries.sort_by_key(|e| e.file_name());

    for entry in &entries {
        let data = fs::read(entry.path()).expect("Failed to read file");
        hasher.update(&data);
    }
    hasher.finalize().to_vec()
}

fn sign(aapp_dir: &str, key_path: &str) {
    let dir = Path::new(aapp_dir);
    if !dir.is_dir() {
        eprintln!("Error: '{}' is not a directory", aapp_dir);
        std::process::exit(1);
    }

    let key_data = fs::read_to_string(key_path)
        .expect("Failed to read private key");
    let key_bytes = base64::engine::general_purpose::STANDARD
        .decode(key_data.trim())
        .expect("Invalid base64 private key");
    let key_array: [u8; 32] = key_bytes.try_into()
        .expect("Private key must be 32 bytes");
    let signing_key = SigningKey::from_bytes(&key_array);

    let hash = hash_aapp_dir(dir);
    let signature: Signature = signing_key.sign(&hash);

    let sig_b64 = base64::engine::general_purpose::STANDARD
        .encode(signature.to_bytes());

    fs::write(dir.join("signature.sig"), &sig_b64)
        .expect("Failed to write signature.sig");

    println!("Signed '{}' successfully (signature.sig written)", aapp_dir);
}

fn verify(aapp_dir: &str, pub_key_path: &str) {
    let dir = Path::new(aapp_dir);
    if !dir.is_dir() {
        eprintln!("Error: '{}' is not a directory", aapp_dir);
        std::process::exit(1);
    }

    let sig_path = dir.join("signature.sig");
    if !sig_path.exists() {
        eprintln!("Error: No signature.sig found in '{}'", aapp_dir);
        std::process::exit(1);
    }

    let pub_key_data = fs::read_to_string(pub_key_path)
        .expect("Failed to read public key");
    let pub_key_bytes = base64::engine::general_purpose::STANDARD
        .decode(pub_key_data.trim())
        .expect("Invalid base64 public key");
    let pub_key_array: [u8; 32] = pub_key_bytes.try_into()
        .expect("Public key must be 32 bytes");
    let verifying_key = VerifyingKey::from_bytes(&pub_key_array)
        .expect("Invalid public key");

    let sig_data = fs::read_to_string(&sig_path)
        .expect("Failed to read signature.sig");
    let sig_bytes = base64::engine::general_purpose::STANDARD
        .decode(sig_data.trim())
        .expect("Invalid base64 signature");
    let sig_array: [u8; 64] = sig_bytes.try_into()
        .expect("Signature must be 64 bytes");
    let signature = Signature::from_bytes(&sig_array);

    let hash = hash_aapp_dir(dir);

    match verifying_key.verify(&hash, &signature) {
        Ok(()) => {
            println!("Signature VALID for '{}'", aapp_dir);
        }
        Err(_) => {
            eprintln!("Signature INVALID for '{}'", aapp_dir);
            std::process::exit(1);
        }
    }
}
