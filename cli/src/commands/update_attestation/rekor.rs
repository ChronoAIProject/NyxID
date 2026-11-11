//! Rekor v1 DSSE verification. In particular, an inclusion proof alone does
//! not authenticate the outer integratedTime. We require a SET before using
//! that time for certificate validation, and verify every supplied proof.
use super::*;
use x509_cert::der::DecodePem;

pub(super) fn verify(bundle: &ParsedBundle, root: &trust::Root) -> Result<UnixTime> {
    let [entry] = bundle.tlog_entries.as_slice() else {
        anyhow::bail!("Expected exactly one Rekor entry");
    };
    let text = |pointer: &str| -> Result<&str> {
        entry
            .pointer(pointer)
            .and_then(Value::as_str)
            .with_context(|| format!("Rekor entry missing {pointer}"))
    };
    let id = normalize_log_key_id(text("/logId/keyId")?)?;
    let key = root.rekor_key(&id)?;
    anyhow::ensure!(
        id == hex::encode(Sha256::digest(&key.der)),
        "Unsupported Rekor v1 log key ID"
    );
    let encoded_body = text("/canonicalizedBody")?;
    anyhow::ensure!(
        encoded_body.len() <= MAX_BUNDLE_BYTES,
        "Rekor body exceeds size limit"
    );
    let body = BASE64.decode(encoded_body)?;
    let body_json: Value = serde_json::from_slice(&body)?;
    anyhow::ensure!(
        entry.pointer("/kindVersion/kind") == body_json.get("kind")
            && entry.pointer("/kindVersion/version") == body_json.get("apiVersion"),
        "Rekor kind/version mismatch"
    );
    bind_dsse(&body_json, bundle)?;

    let time = parse_json_u64(&entry["integratedTime"])?;
    let index = parse_json_u64(&entry["logIndex"])?;
    // SET payload is the Rekor JSON canonical form. These four ASCII keys and
    // unsigned integers have a unique JCS encoding; body is the exact logged
    // bytes, not a re-serialization of parsed JSON.
    let signed = format!(
        "{{\"body\":\"{}\",\"integratedTime\":{time},\"logID\":\"{id}\",\"logIndex\":{index}}}",
        BASE64.encode(&body)
    );
    let set = BASE64
        .decode(text("/inclusionPromise/signedEntryTimestamp")?)
        .context("Rekor v1 requires an authenticated SET signing time")?;
    let verifier = CosignVerificationKey::try_from_der(&key.der)?;
    verifier
        .verify_signature(Signature::Raw(&set), signed.as_bytes())
        .context("Rekor SET signature verification failed")?;
    key.check_time(time)?;
    anyhow::ensure!(
        time <= (chrono::Utc::now().timestamp() + 300) as u64,
        "Rekor signing time is in the future"
    );

    match entry.get("inclusionProof") {
        Some(proof) => verify_proof(proof, &body, key, &verifier)?,
        None => anyhow::ensure!(
            !bundle.require_proof,
            "Bundle format requires an inclusion proof"
        ),
    }
    Ok(UnixTime::since_unix_epoch(Duration::from_secs(time)))
}

fn bind_dsse(body: &Value, bundle: &ParsedBundle) -> Result<()> {
    anyhow::ensure!(
        body["kind"] == "dsse" && body["apiVersion"] == "0.0.1",
        "Unsupported Rekor entry kind/version"
    );
    anyhow::ensure!(
        bundle.envelope.payload_type == "application/vnd.in-toto+json"
            && bundle.envelope.signatures.len() == 1,
        "Expected one in-toto DSSE signature"
    );
    let spec = &body["spec"];
    check_hash(&spec["payloadHash"], &bundle.payload)?;
    // GitHub's producer serializes the DSSE envelope using the field order
    // below, including an empty keyid. Accept only this explicit producer
    // encoding rather than pretending arbitrary original JSON is recoverable.
    check_hash(
        &spec["envelopeHash"],
        &serde_json::to_vec(&bundle.envelope)?,
    )?;
    let signatures = spec["signatures"]
        .as_array()
        .context("Missing logged signatures")?;
    anyhow::ensure!(signatures.len() == 1, "Expected one logged signature");
    let signature = &signatures[0];
    let logged_sig = BASE64.decode(
        signature["signature"]
            .as_str()
            .context("Missing logged signature")?,
    )?;
    anyhow::ensure!(
        logged_sig == BASE64.decode(&bundle.envelope.signatures[0].sig)?,
        "Logged DSSE signature mismatch"
    );
    let pem = BASE64.decode(
        signature["verifier"]
            .as_str()
            .context("Missing logged certificate")?,
    )?;
    let cert = Certificate::from_pem(&pem).context("Malformed logged certificate")?;
    anyhow::ensure!(
        cert.to_der()? == bundle.certificate_chain[0],
        "Logged certificate mismatch"
    );
    Ok(())
}

fn check_hash(hash: &Value, bytes: &[u8]) -> Result<()> {
    anyhow::ensure!(
        hash["algorithm"] == "sha256"
            && hash["value"].as_str() == Some(hex::encode(Sha256::digest(bytes)).as_str()),
        "Rekor DSSE hash mismatch"
    );
    Ok(())
}

fn hash_bytes(value: &Value) -> Result<[u8; 32]> {
    BASE64
        .decode(value.as_str().context("Missing Merkle hash")?)?
        .try_into()
        .map_err(|_| anyhow::anyhow!("Merkle hash must be 32 bytes"))
}

fn verify_proof(
    proof: &Value,
    body: &[u8],
    key: &trust::LogKey,
    verifier: &CosignVerificationKey,
) -> Result<()> {
    let index = parse_json_u64(&proof["logIndex"])?;
    let size = parse_json_u64(&proof["treeSize"])?;
    anyhow::ensure!(size > 0 && index < size, "Invalid Merkle index/tree size");
    let root = hash_bytes(&proof["rootHash"])?;
    let hashes = proof["hashes"].as_array().context("Missing Merkle proof")?;
    anyhow::ensure!(hashes.len() <= 64, "Oversized Merkle proof");
    let hashes = hashes.iter().map(hash_bytes).collect::<Result<Vec<_>>>()?;
    verify_merkle(body, index, size, &root, &hashes)?;
    // Rekor shards use a virtual outer logIndex for SETs and a shard-local
    // proof index. Their numeric equality is not a valid consistency check.
    verify_checkpoint(
        proof["checkpoint"]["envelope"]
            .as_str()
            .context("Missing signed checkpoint")?,
        size,
        &root,
        key,
        verifier,
    )
}

fn verify_merkle(
    body: &[u8],
    index: u64,
    size: u64,
    root: &[u8; 32],
    hashes: &[[u8; 32]],
) -> Result<()> {
    anyhow::ensure!(
        size > 0 && index < size && hashes.len() <= 64,
        "Invalid Merkle proof bounds"
    );
    let mut digest: [u8; 32] = Sha256::digest([&[0u8][..], body].concat()).into();
    let (mut leaf, mut last) = (index, size - 1);
    for hash in hashes {
        anyhow::ensure!(last != 0, "Merkle proof contains extra nodes");
        let sibling = hash;
        if leaf & 1 == 1 || leaf == last {
            digest = node(sibling, &digest);
            while leaf != 0 && leaf & 1 == 0 {
                leaf >>= 1;
                last >>= 1;
            }
        } else {
            digest = node(&digest, sibling);
        }
        leaf >>= 1;
        last >>= 1;
    }
    anyhow::ensure!(
        leaf == 0 && last == 0 && digest == *root,
        "Merkle inclusion proof mismatch"
    );
    Ok(())
}

fn node(left: &[u8; 32], right: &[u8; 32]) -> [u8; 32] {
    let mut h = Sha256::new();
    h.update([1]);
    h.update(left);
    h.update(right);
    h.finalize().into()
}

fn verify_checkpoint(
    envelope: &str,
    size: u64,
    root: &[u8; 32],
    key: &trust::LogKey,
    verifier: &CosignVerificationKey,
) -> Result<()> {
    anyhow::ensure!(envelope.len() <= 8192, "Checkpoint exceeds size limit");
    let (body, signatures) = envelope
        .split_once("\n\n")
        .context("Malformed signed checkpoint")?;
    let lines: Vec<_> = body.lines().collect();
    anyhow::ensure!(
        lines.len() == 3
            && (lines[0] == key.origin
                || lines[0]
                    .strip_prefix(&format!("{} - ", key.origin))
                    .is_some_and(
                        |shard| !shard.is_empty() && shard.bytes().all(|c| c.is_ascii_digit())
                    )),
        "Checkpoint origin mismatch"
    );
    anyhow::ensure!(
        lines[1] == size.to_string() && BASE64.decode(lines[2])? == root,
        "Checkpoint tree size/root mismatch"
    );
    let signed = format!("{body}\n");
    let id = hex::decode(&key.id)?;
    let mut valid = false;
    for line in signatures.lines() {
        let fields: Vec<_> = line.split(' ').collect();
        anyhow::ensure!(
            fields.len() == 3 && fields[0] == "—",
            "Malformed checkpoint signature"
        );
        let bytes = BASE64.decode(fields[2])?;
        if fields[1] == key.origin && bytes.len() > 4 && bytes[..4] == id[..4] {
            verifier
                .verify_signature(Signature::Raw(&bytes[4..]), signed.as_bytes())
                .context("Checkpoint signature verification failed")?;
            valid = true;
        }
    }
    anyhow::ensure!(valid, "Checkpoint has no signature from the expected log");
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn subtree(leaves: &[[u8; 32]]) -> [u8; 32] {
        if leaves.len() == 1 {
            return leaves[0];
        }
        let split = 1 << ((leaves.len() - 1).ilog2());
        node(&subtree(&leaves[..split]), &subtree(&leaves[split..]))
    }

    // RFC6962's recursive audit-path definition is independent of the verifier's
    // iterative index traversal, exercising every branch on irregular trees.
    fn path(leaves: &[[u8; 32]], index: usize) -> Vec<[u8; 32]> {
        if leaves.len() == 1 {
            return vec![];
        }
        let split = 1 << ((leaves.len() - 1).ilog2());
        if index < split {
            let mut proof = path(&leaves[..split], index);
            proof.push(subtree(&leaves[split..]));
            proof
        } else {
            let mut proof = path(&leaves[split..], index - split);
            proof.push(subtree(&leaves[..split]));
            proof
        }
    }

    #[test]
    fn rfc6962_paths_cover_single_leaf_power_of_two_and_irregular_trees() {
        for size in 1..=33 {
            let bodies: Vec<_> = (0..size)
                .map(|i| format!("entry {i}").into_bytes())
                .collect();
            let leaves: Vec<[u8; 32]> = bodies
                .iter()
                .map(|b| Sha256::digest([&[0][..], b.as_slice()].concat()).into())
                .collect();
            let root = subtree(&leaves);
            for (index, body) in bodies.iter().enumerate() {
                let proof = path(&leaves, index);
                verify_merkle(body, index as u64, size as u64, &root, &proof).unwrap();
                assert!(
                    verify_merkle(b"different leaf", index as u64, size as u64, &root, &proof)
                        .is_err()
                );
                let mut extra = proof.clone();
                extra.push(root);
                assert!(verify_merkle(body, index as u64, size as u64, &root, &extra).is_err());
                if !proof.is_empty() {
                    assert!(
                        verify_merkle(
                            body,
                            index as u64,
                            size as u64,
                            &root,
                            &proof[..proof.len() - 1]
                        )
                        .is_err()
                    );
                }
            }
        }
        assert!(verify_merkle(b"x", 0, 0, &[0; 32], &[]).is_err());
        assert!(verify_merkle(b"x", 1, 1, &[0; 32], &[]).is_err());
    }
}
