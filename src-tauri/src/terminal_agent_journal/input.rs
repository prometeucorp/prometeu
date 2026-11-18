//! A durable input intent authorizes one write attempt, never a retry after uncertain acceptance.
use super::*;

const INPUT_LIMIT: usize = 2 * 1024 * 1024;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct PreparedInput {
    pub id: String,
    pub original: String,
    pub original_hash: String,
    pub normalized_hash: String,
    pub after_native_user: Option<String>,
    pub empty_baseline: bool,
    pub at: u64,
}

impl CaptureJournal {
    /// `Ok` authorizes only this caller's first write attempt. An existing unresolved intent is
    /// uncertainty, even when its text is identical; acceptance must be reconciled from source.
    pub(crate) fn prepare_input(&mut self, input: &PreparedInput) -> Result<(), JournalError> {
        validate_input(input)?;
        if self
            .lookup(&format!("app:submission:{}", input.id), "accepted")?
            .is_some()
        {
            return Err(JournalError::Conflict);
        }
        let transaction = self
            .connection
            .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)
            .map_err(|_| JournalError::Storage)?;
        let pending: i64 = transaction
            .query_row("SELECT count(*) FROM prepared_inputs", [], |row| row.get(0))
            .map_err(|_| JournalError::Storage)?;
        if pending != 0 {
            return Err(JournalError::Conflict);
        }
        transaction
            .execute(
                "INSERT INTO prepared_inputs VALUES (?1,?2)",
                rusqlite::params![
                    input.id,
                    serde_json::to_string(input).map_err(|_| JournalError::Invalid)?
                ],
            )
            .map_err(|_| JournalError::Storage)?;
        transaction.commit().map_err(|_| JournalError::Storage)
    }

    pub(crate) fn prepared_inputs(&self) -> Result<Vec<PreparedInput>, JournalError> {
        let mut query = self
            .connection
            .prepare("SELECT substr(value,1,2097153) FROM prepared_inputs ORDER BY rowid LIMIT 2")
            .map_err(|_| JournalError::Storage)?;
        let rows = query
            .query_map([], |row| row.get::<_, String>(0))
            .map_err(|_| JournalError::Storage)?;
        let mut inputs = Vec::new();
        for row in rows {
            let row = row.map_err(|_| JournalError::Storage)?;
            if row.len() > INPUT_LIMIT || !inputs.is_empty() {
                return Err(JournalError::Invalid);
            }
            let input: PreparedInput =
                serde_json::from_str(&row).map_err(|_| JournalError::Invalid)?;
            validate_input(&input)?;
            inputs.push(input);
        }
        Ok(inputs)
    }

    /// Remove the transient original text only after its canonical echo owns a durable receipt.
    pub(crate) fn finish_input(&self, id: &str) -> Result<(), JournalError> {
        if self
            .lookup(&format!("app:submission:{id}"), "accepted")?
            .is_none()
        {
            return Err(JournalError::Conflict);
        }
        self.connection
            .execute("DELETE FROM prepared_inputs WHERE id=?1", [id])
            .map_err(|_| JournalError::Storage)?;
        Ok(())
    }

    pub(crate) fn recover_inputs(&self) -> Result<(), JournalError> {
        for input in self.prepared_inputs()? {
            if self
                .lookup(&format!("app:submission:{}", input.id), "accepted")?
                .is_some()
            {
                self.finish_input(&input.id)?;
            }
        }
        Ok(())
    }

    /// The host must prove that its still-reserved input never attempted a process write.
    pub(crate) fn cancel_unwritten_input(&self, id: &str) -> Result<(), JournalError> {
        if !valid_id(id)
            || self
                .lookup(&format!("app:submission:{id}"), "accepted")?
                .is_some()
        {
            return Err(JournalError::Conflict);
        }
        self.connection
            .execute("DELETE FROM prepared_inputs WHERE id=?1", [id])
            .map_err(|_| JournalError::Storage)?;
        Ok(())
    }
}

fn validate_input(input: &PreparedInput) -> Result<(), JournalError> {
    let content = serde_json::json!([{"kind":"text","text":input.original}]).to_string();
    let original_hash = format!("{:x}", Sha256::digest(content.as_bytes()));
    if !valid_id(&input.id)
        || input.original.len() > 1024 * 1024
        || input.original_hash.len() != 64
        || input.original_hash != original_hash
        || input.normalized_hash.len() != 64
        || !input
            .original_hash
            .bytes()
            .chain(input.normalized_hash.bytes())
            .all(|byte| byte.is_ascii_hexdigit())
        || input
            .after_native_user
            .as_deref()
            .is_some_and(|key| !valid_key(key))
        || serde_json::to_vec(input)
            .map_err(|_| JournalError::Invalid)?
            .len()
            > INPUT_LIMIT
    {
        return Err(JournalError::Invalid);
    }
    Ok(())
}
