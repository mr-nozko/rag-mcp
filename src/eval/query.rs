//! Eval query type and relevance logic for the evaluation framework.

use crate::db::Db;
use crate::error::Result;
use crate::search::SearchResult;
use serde::Deserialize;

/// Single evaluation query with optional relevance criteria.
#[derive(Debug, Clone, Deserialize)]
pub struct EvalQuery {
    /// Query text to run against hybrid search.
    pub query: String,
    /// Category for reporting (e.g. agent_identity, routing, playbooks, system).
    pub category: String,
    /// Optional: consider chunks from this document path relevant (path match).
    #[serde(default)]
    pub expected_doc: Option<String>,
    /// Optional: further restrict relevance to this section header.
    #[serde(default)]
    pub expected_section: Option<String>,
    /// Optional: for entity/routing queries, agent names (e.g. "agent:dev") for is_relevant.
    #[serde(default)]
    pub expected_entities: Option<Vec<String>>,
    /// Optional: minimum rank requirement (for future use).
    #[serde(default)]
    pub min_rank: Option<usize>,
    /// Optional: explicit list of relevant chunk IDs (overrides DB lookup when non-empty).
    #[serde(default)]
    pub relevant_chunk_ids: Option<Vec<String>>,
}

impl EvalQuery {
    /// Returns the set of chunk IDs considered relevant for this query.
    /// Uses explicit relevant_chunk_ids from JSON if present; otherwise resolves from
    /// expected_doc (and optional expected_section) via DB lookup.
    pub async fn relevant_chunk_ids(&self, db: &Db) -> Result<Vec<String>> {
        // If explicit list provided, use it
        if let Some(ref ids) = self.relevant_chunk_ids {
            if !ids.is_empty() {
                return Ok(ids.clone());
            }
        }
        // Resolve from expected_doc + optional expected_section
        if let Some(ref doc_path) = self.expected_doc {
            // Normalize path separators so we match DB (Windows may store backslashes)
            let doc_path = doc_path.replace('\\', "/").trim_matches('/').to_string();
            let section = self.expected_section.clone();
            let ids = db
                .with_connection(move |conn| {
                    // Split expected_doc by '/' and match if all parts appear in doc_path (order-independent)
                    // This handles variations like "module-alpha/overview.md" matching "docs_module-alpha_overview.md"
                    let doc_parts: Vec<String> = doc_path.split('/').filter(|s| !s.is_empty()).map(String::from).collect();
                    let mut conditions = vec![];
                    for _ in &doc_parts {
                        conditions.push("REPLACE(d.doc_path, '\\', '/') LIKE '%' || ? || '%'");
                    }
                    let where_clause = conditions.join(" AND ");
                    let sql = format!(
                        "SELECT c.chunk_id FROM chunks c INNER JOIN documents d ON c.doc_id = d.doc_id WHERE {} {}",
                        where_clause,
                        if section.is_some() { "AND c.section_header = ?" } else { "" }
                    );
                    
                    let mut stmt = conn.prepare(&sql)?;
                    // Build params: each doc part, then optional section
                    let mut param_refs: Vec<&dyn rusqlite::ToSql> = doc_parts.iter().map(|p| p as &dyn rusqlite::ToSql).collect();
                    if let Some(ref s) = section {
                        param_refs.push(s);
                    }
                    let rows = stmt.query_map(param_refs.as_slice(), |row| row.get::<_, String>(0))?;
                    let mut out = Vec::new();
                    for row in rows {
                        out.push(row?);
                    }
                    Ok(out)
                })
                .await?;
            return Ok(ids);
        }
        Ok(Vec::new())
    }

    /// Returns true if the given search result is considered relevant for this query.
    /// Matches on expected_doc (path), expected_section (section), and expected_entities (agent_name).
    /// If no criteria are set, returns false (avoids inflating MRR for queries without ground truth).
    pub fn is_relevant(&self, result: &SearchResult) -> bool {
        let has_doc = self.expected_doc.is_some();
        let has_section = self.expected_section.is_some();
        let has_entities = self
            .expected_entities
            .as_ref()
            .map(|e| !e.is_empty())
            .unwrap_or(false);
        if !has_doc && !has_section && !has_entities {
            return false;
        }
        // Doc path: flexible substring match (handles path variations like docs_module-alpha_overview.md vs prompt.xml)
        if let Some(ref expected) = self.expected_doc {
            let exp = expected.replace('\\', "/").trim_matches('/').to_string();
            let got_norm = result.doc_path.replace('\\', "/").to_lowercase();
            let exp_lower = exp.to_lowercase();
            // Match if expected path is contained in actual path (e.g. "module-alpha/overview.md" in "agents/module-alpha/docs_module-alpha_overview.md")
            if !got_norm.contains(&exp_lower) {
                return false;
            }
        }
        // Section: exact match if expected_section set
        if let Some(ref expected) = self.expected_section {
            match &result.section {
                Some(s) if s == expected => {}
                _ => return false,
            }
        }
        // Entities: result.agent_name in expected_entities (strip "agent:" prefix from expected)
        if let Some(ref entities) = self.expected_entities {
            let agent = match &result.agent_name {
                Some(a) => a.as_str(),
                None => return false,
            };
            let match_ = entities.iter().any(|e| {
                let normalized = e.strip_prefix("agent:").unwrap_or(e);
                agent == normalized || agent == e
            });
            if !match_ {
                return false;
            }
        }
        true
    }
}
