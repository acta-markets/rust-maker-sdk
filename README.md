# acta-maker-sdk

Rust SDK for Acta options market makers: WebSocket clients, quote signing and
Solana funding instructions.

Managed clients handle authentication, reconnects, subscriptions and response
correlation. Your application provides pricing and manages its orders and risk.

## Install

Requires Rust 1.89 or later.

```toml
[dependencies]
acta-maker-sdk = { version = "0.4.2", features = ["ws-client"] }
```

Use `MakerQuoteClient` to submit, replace and cancel quotes, and `MakerDataClient`
to read maker state. Maker registration is required for authentication.

Default features are empty. Enable `ws-client` for WebSocket clients,
`chain` for Solana instruction builders, or `chain-rpc` for RPC helpers.

## Documentation

- [Integration documentation](https://beta.acta.markets/docs)
- [Examples](https://github.com/acta-markets/rust-maker-sdk/tree/main/examples)
- [Changelog](https://github.com/acta-markets/rust-maker-sdk/blob/main/CHANGELOG.md)

## License

MIT
