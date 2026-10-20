use acta_maker_sdk::{BytesSigner, decode_base58_32};
use zeroize::Zeroizing;

pub struct Config {
    pub signer: BytesSigner,
    /// `ACTA_MAKER_OWNER`: the registered maker owner, required for maker sessions.
    pub owner: Option<String>,
    pub url: String,
}

pub fn load() -> Result<Config, Box<dyn std::error::Error>> {
    let path = std::env::var("ACTA_KEYPAIR_PATH")
        .map_err(|_| "set ACTA_KEYPAIR_PATH to your registered maker signing-key JSON file")?;
    let json = Zeroizing::new(std::fs::read(path)?);
    let bytes = Zeroizing::new(serde_json::from_slice::<Vec<u8>>(&json)?);
    let keypair: &[u8; 64] = bytes
        .as_slice()
        .try_into()
        .map_err(|_| "expected a Solana keypair JSON array of 64 bytes")?;
    let signer = BytesSigner::from_keypair(keypair)?;
    let owner = match std::env::var("ACTA_MAKER_OWNER") {
        Ok(owner) => {
            decode_base58_32(&owner)?;
            Some(owner)
        }
        Err(std::env::VarError::NotPresent) => None,
        Err(error) => return Err(error.into()),
    };
    let url = match std::env::var("ACTA_WS_URL") {
        Ok(url) => url,
        Err(std::env::VarError::NotPresent) => acta_maker_sdk::DEVNET_WS.into(),
        Err(error) => return Err(error.into()),
    };
    Ok(Config { signer, owner, url })
}
