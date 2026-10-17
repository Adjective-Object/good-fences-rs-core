use eyre::ensure;

/// Preflight lengths and nesting before serde can reserve storage from input.
pub(crate) fn check(bytes: &[u8]) -> eyre::Result<()> {
    ensure!(
        bytes.len() <= crate::MAX_RECORD_BYTES,
        "cache exceeds byte limit"
    );
    let mut cursor = 0;
    let mut remaining = 262_144;
    item(bytes, &mut cursor, &mut remaining, 0)?;
    ensure!(cursor == bytes.len(), "trailing cache data");
    Ok(())
}

fn item(bytes: &[u8], cursor: &mut usize, budget: &mut usize, depth: usize) -> eyre::Result<()> {
    ensure!(depth <= 32 && *budget > 0, "cache exceeds structure limit");
    *budget -= 1;
    let header = *bytes
        .get(*cursor)
        .ok_or_else(|| eyre::eyre!("truncated CBOR"))?;
    *cursor += 1;
    let additional = header & 31;
    let value = match additional {
        0..=23 => u64::from(additional),
        24..=27 => {
            let count = 1usize << (additional - 24);
            let end = cursor
                .checked_add(count)
                .ok_or_else(|| eyre::eyre!("CBOR overflow"))?;
            let slice = bytes
                .get(*cursor..end)
                .ok_or_else(|| eyre::eyre!("truncated CBOR"))?;
            *cursor = end;
            slice.iter().fold(0u64, |n, b| (n << 8) | u64::from(*b))
        }
        _ => eyre::bail!("indefinite/reserved CBOR length"),
    };
    match header >> 5 {
        0 | 1 => {}
        2 | 3 => {
            let length = usize::try_from(value)?;
            let end = cursor
                .checked_add(length)
                .ok_or_else(|| eyre::eyre!("CBOR overflow"))?;
            ensure!(end <= bytes.len(), "truncated CBOR string");
            *cursor = end;
        }
        4 | 5 => {
            let count = if header >> 5 == 5 {
                value
                    .checked_mul(2)
                    .ok_or_else(|| eyre::eyre!("CBOR overflow"))?
            } else {
                value
            };
            ensure!(count <= *budget as u64, "cache exceeds item limit");
            for _ in 0..count {
                item(bytes, cursor, budget, depth + 1)?;
            }
        }
        7 => ensure!(
            matches!(additional, 20..=22),
            "unsupported CBOR simple value"
        ),
        _ => eyre::bail!("unsupported CBOR tag"),
    }
    Ok(())
}
