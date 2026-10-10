use solana_sdk::pubkey::Pubkey;

pub(crate) fn global_config(program_id: &Pubkey) -> (Pubkey, u8) {
    Pubkey::find_program_address(&[b"global_config"], program_id)
}

pub(crate) fn oracle_source(program_id: &Pubkey, mint: &Pubkey) -> (Pubkey, u8) {
    Pubkey::find_program_address(&[b"oracle_source", mint.as_ref()], program_id)
}

pub(crate) fn vault(program_id: &Pubkey, vault_id: &[u8; 32]) -> (Pubkey, u8) {
    Pubkey::find_program_address(&[b"vault", vault_id], program_id)
}

pub(crate) fn vault_policy(program_id: &Pubkey, vault: &Pubkey) -> (Pubkey, u8) {
    Pubkey::find_program_address(&[b"vault_policy", vault.as_ref()], program_id)
}

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

pub(crate) fn manager_stake(program_id: &Pubkey, vault: &Pubkey) -> (Pubkey, u8) {
    Pubkey::find_program_address(&[b"manager_stake", vault.as_ref()], program_id)
}

pub(crate) fn deposit_request(program_id: &Pubkey, vault: &Pubkey, user: &Pubkey) -> (Pubkey, u8) {
    Pubkey::find_program_address(
        &[b"deposit_request", vault.as_ref(), user.as_ref()],
        program_id,
    )
}

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

pub(crate) fn withdraw_request(program_id: &Pubkey, vault: &Pubkey, user: &Pubkey) -> (Pubkey, u8) {
    Pubkey::find_program_address(
        &[b"withdraw_request", vault.as_ref(), user.as_ref()],
        program_id,
    )
}

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

pub(crate) fn pending_vault_update(program_id: &Pubkey, vault: &Pubkey, kind: u8) -> (Pubkey, u8) {
    Pubkey::find_program_address(&[b"pending_update", vault.as_ref(), &[kind]], program_id)
}

pub(crate) fn market(
    program_id: &Pubkey,
    underlying_mint: &Pubkey,
    quote_mint: &Pubkey,
    expiry_ts: u64,
    is_put: bool,
) -> (Pubkey, u8) {
    Pubkey::find_program_address(
        &[
            b"market",
            underlying_mint.as_ref(),
            quote_mint.as_ref(),
            &expiry_ts.to_le_bytes(),
            &[u8::from(is_put)],
        ],
        program_id,
    )
}

pub(crate) fn position(program_id: &Pubkey, order_id: &[u8; 32]) -> (Pubkey, u8) {
    Pubkey::find_program_address(&[b"position", order_id], program_id)
}

pub(crate) fn premium_config(program_id: &Pubkey, premium_mint: &Pubkey) -> (Pubkey, u8) {
    Pubkey::find_program_address(&[b"config", premium_mint.as_ref()], program_id)
}
