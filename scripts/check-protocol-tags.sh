#!/usr/bin/env bash
set -euo pipefail

sdk_dir="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
backend_dir="${ACTA_BACKEND_DIR:-"$sdk_dir/../rust-backend"}"
backend_protocol_dir="$backend_dir/acta-ws-protocol/src"

if [[ ! -f "$backend_protocol_dir/client.rs" || ! -f "$backend_protocol_dir/server.rs" ]]; then
  echo "Acta backend protocol sources not found at $backend_protocol_dir" >&2
  echo "Set ACTA_BACKEND_DIR to the rust-backend repository root." >&2
  exit 2
fi

tmp_dir="$(mktemp -d)"
trap 'rm -rf "$tmp_dir"' EXIT

extract_variants() {
  local enum_name="$1"
  local source_file="$2"
  awk -v target="$enum_name" '
    $0 ~ "pub enum " target "[ <{]" { inside = 1; next }
    inside && /^}/ { exit }
    inside && /^    [A-Z][A-Za-z0-9_]*/ {
      line = $0
      sub(/^    /, "", line)
      sub(/[(,{].*$/, "", line)
      print line
    }
  ' "$source_file" | sort
}

# `Unknown` is an SDK-only decoding fallback, not a wire value emitted by the backend.
compare_enum() {
  local enum_name="$1"
  local sdk_file="$2"
  local backend_file="$3"
  local retain_unknown="${4:-false}"
  if [[ "$retain_unknown" == true ]]; then
    extract_variants "$enum_name" "$sdk_file" > "$tmp_dir/sdk-$enum_name"
  else
    extract_variants "$enum_name" "$sdk_file" \
      | sed '/^Unknown$/d' > "$tmp_dir/sdk-$enum_name"
  fi
  extract_variants "$enum_name" "$backend_file" \
    > "$tmp_dir/backend-$enum_name"
  if [[ ! -s "$tmp_dir/sdk-$enum_name" || ! -s "$tmp_dir/backend-$enum_name" ]]; then
    echo "extract_variants found no variants for $enum_name (renamed or moved?)" >&2
    exit 1
  fi
  diff -u --label "backend-$enum_name" --label "sdk-$enum_name" \
    "$tmp_dir/backend-$enum_name" "$tmp_dir/sdk-$enum_name"
}

backend_types_dir="$backend_dir/acta-types/src"

compare_enum ClientMessage "$sdk_dir/src/ws/types/client.rs" "$backend_protocol_dir/client.rs"
compare_enum ServerMessage "$sdk_dir/src/ws/types/server/mod.rs" "$backend_protocol_dir/server.rs"
compare_enum ServerError "$sdk_dir/src/ws/types/server/mod.rs" "$backend_protocol_dir/server.rs"
compare_enum BatchQuoteResult "$sdk_dir/src/ws/types/server/server_quote.rs" "$backend_protocol_dir/server.rs"
compare_enum QuoteRejectReason "$sdk_dir/src/ws/types/server/server_quote.rs" "$backend_protocol_dir/server.rs"
compare_enum WsChannel "$sdk_dir/src/ws/types/common.rs" "$backend_protocol_dir/common.rs"
compare_enum MakerQuoteScope "$sdk_dir/src/ws/types/client_query.rs" "$backend_protocol_dir/client.rs"
for quote_enum in QuoteRank HistoricalQuoteStatus MakerQuoteState; do
  compare_enum "$quote_enum" "$sdk_dir/src/ws/types/server/server_query.rs" "$backend_types_dir/messages.rs"
done
compare_enum OrderExecutionState "$sdk_dir/src/ws/types/server/server_query.rs" "$backend_protocol_dir/server.rs" true
for payload_enum in RateLimitReason CapError UserRole AuthRequiredAction RfqStateError QuoteLockedReason DbFeature; do
  compare_enum "$payload_enum" "$sdk_dir/src/types/errors.rs" "$backend_types_dir/errors.rs"
done

python3 "$backend_dir/scripts/check-wire-drift.py" --sdk "$sdk_dir"

echo "SDK and backend protocol enums and fields match."
