//! Program-derived addresses of the vault program surface.

use solana_sdk::pubkey::Pubkey;

/// Global config PDA.
pub(crate) fn global_config(program_id: &Pubkey) -> (Pubkey, u8) {
    Pubkey::find_program_address(&[b"global_config"], program_id)
}

/// Oracle source PDA for a mint.
pub(crate) fn oracle_source(program_id: &Pubkey, mint: &Pubkey) -> (Pubkey, u8) {
    Pubkey::find_program_address(&[b"oracle_source", mint.as_ref()], program_id)
}

/// Vault PDA for a vault id.
pub(crate) fn vault(program_id: &Pubkey, vault_id: &[u8; 32]) -> (Pubkey, u8) {
    Pubkey::find_program_address(&[b"vault", vault_id], program_id)
}

/// Vault policy PDA.
pub(crate) fn vault_policy(program_id: &Pubkey, vault: &Pubkey) -> (Pubkey, u8) {
    Pubkey::find_program_address(&[b"vault_policy", vault.as_ref()], program_id)
}

/// Vault delegate PDA for a delegate authority.
pub(crate) fn vault_delegate(
    program_id: &Pubkey,
    vault: &Pubkey,
    delegate: &Pubkey,
) -> (Pubkey, u8) {
    Pubkey::find_program_address(
        &[b"vault_delegate", vault.as_ref(), delegate.as_ref()],
        program_id,
    )
}

/// Manager stake authority PDA.
pub(crate) fn manager_stake(program_id: &Pubkey, vault: &Pubkey) -> (Pubkey, u8) {
    Pubkey::find_program_address(&[b"manager_stake", vault.as_ref()], program_id)
}

/// Deposit request PDA of a user.
pub(crate) fn deposit_request(program_id: &Pubkey, vault: &Pubkey, user: &Pubkey) -> (Pubkey, u8) {
    Pubkey::find_program_address(
        &[b"deposit_request", vault.as_ref(), user.as_ref()],
        program_id,
    )
}

/// Deposit escrow authority PDA of a user.
pub(crate) fn deposit_escrow_authority(
    program_id: &Pubkey,
    vault: &Pubkey,
    user: &Pubkey,
) -> (Pubkey, u8) {
    Pubkey::find_program_address(
        &[b"deposit_escrow", vault.as_ref(), user.as_ref()],
        program_id,
    )
}

/// Withdraw request PDA of a user.
pub(crate) fn withdraw_request(program_id: &Pubkey, vault: &Pubkey, user: &Pubkey) -> (Pubkey, u8) {
    Pubkey::find_program_address(
        &[b"withdraw_request", vault.as_ref(), user.as_ref()],
        program_id,
    )
}

/// Withdraw escrow authority PDA of a user.
pub(crate) fn withdraw_escrow_authority(
    program_id: &Pubkey,
    vault: &Pubkey,
    user: &Pubkey,
) -> (Pubkey, u8) {
    Pubkey::find_program_address(
        &[b"withdraw_escrow", vault.as_ref(), user.as_ref()],
        program_id,
    )
}

/// Pending governance update PDA for an update kind.
pub(crate) fn pending_vault_update(program_id: &Pubkey, vault: &Pubkey, kind: u8) -> (Pubkey, u8) {
    Pubkey::find_program_address(&[b"pending_update", vault.as_ref(), &[kind]], program_id)
}
