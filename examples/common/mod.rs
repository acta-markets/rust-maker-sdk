use acta_maker_sdk::{BytesSigner, SignerLike, decode_base58_32};
use zeroize::Zeroizing;

pub struct Config {
    pub signer: BytesSigner,
    pub owner: String,
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
        Ok(owner) => owner,
        Err(std::env::VarError::NotPresent) => signer.pubkey_base58(),
        Err(error) => return Err(error.into()),
    };
    decode_base58_32(&owner)?;
    let url = match std::env::var("ACTA_WS_URL") {
        Ok(url) => url,
        Err(std::env::VarError::NotPresent) => "wss://devnet-api.acta.markets".into(),
        Err(error) => return Err(error.into()),
    };
    Ok(Config { signer, owner, url })
}
