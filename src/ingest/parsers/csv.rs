use super::{Parser, ParsedDocument, Section};
use crate::error::{Result, RagmcpError};

/// CSV parser: one search section per data row, with column names kept beside values.
///
/// Language tables and logic matrices are source-of-truth rows. Indexing the file
/// as one blob would bury a gloss or matrix cell inside a 300-token window.
/// A header row is required; the first data row is treated as headers if none
/// is marked (csv crate `has_headers(true)`).
pub struct CsvParser;

impl Parser for CsvParser {
    fn can_parse(&self, extension: &str) -> bool {
        extension == "csv"
    }

    fn parse(&self, content: &str, path: &str) -> Result<ParsedDocument> {
        // Excel often writes UTF-8 BOM; leaving it would become part of column 1.
        let stripped = content.strip_prefix('\u{FEFF}').unwrap_or(content);

        // ::csv is the crate — this module is also named csv.
        let mut reader = ::csv::ReaderBuilder::new()
            .has_headers(true)
            .from_reader(stripped.as_bytes());

        let headers = reader
            .headers()
            .map_err(|e| RagmcpError::Parse(format!("CSV parse error in {}: {}", path, e)))?
            .clone();

        let mut sections = Vec::new();
        for (i, record) in reader.records().enumerate() {
            let record = record.map_err(|e| {
                RagmcpError::Parse(format!("CSV parse error in {}: {}", path, e))
            })?;

            // Blank lines are not source-of-truth rows.
            if record.iter().all(|cell| cell.trim().is_empty()) {
                continue;
            }

            // 1-based among data records so an empty first cell still has a stable title.
            let row_num = i + 1;
            let first = record.get(0).unwrap_or("").trim();
            let header = if first.is_empty() {
                format!("row {}", row_num)
            } else {
                first.to_string()
            };

            let mut lines = Vec::with_capacity(headers.len());
            for (col_idx, col_name) in headers.iter().enumerate() {
                let value = record.get(col_idx).unwrap_or("");
                lines.push(format!("{}: {}", col_name, value));
            }

            sections.push(Section {
                header,
                content: lines.join("\n"),
                section_type: Some("csv_row".to_string()),
            });
        }

        // Document body is the labeled records so content_text stays searchable as rows.
        let content_text = sections
            .iter()
            .map(|s| s.content.as_str())
            .collect::<Vec<_>>()
            .join("\n\n");

        Ok(ParsedDocument {
            content: content_text,
            sections,
            doc_type: "csv".to_string(),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(input: &str) -> ParsedDocument {
        CsvParser.parse(input, "table.csv").expect("parse")
    }

    #[test]
    fn test_two_rows_keep_column_names() {
        let doc = parse("term,language,gloss\nhello,en,greeting\nciao,it,greeting\n");
        assert_eq!(doc.doc_type, "csv");
        assert_eq!(doc.sections.len(), 2);
        assert_eq!(doc.sections[0].header, "hello");
        assert_eq!(doc.sections[0].section_type.as_deref(), Some("csv_row"));
        assert_eq!(
            doc.sections[0].content,
            "term: hello\nlanguage: en\ngloss: greeting"
        );
        assert_eq!(doc.sections[1].header, "ciao");
        assert!(doc.content.contains("term: hello"));
        assert!(doc.content.contains("term: ciao"));
    }

    #[test]
    fn test_quoted_comma_stays_in_one_cell() {
        let doc = parse("term,gloss\n\"hello, world\",greeting\n");
        assert_eq!(doc.sections.len(), 1);
        assert_eq!(doc.sections[0].content, "term: hello, world\ngloss: greeting");
    }

    #[test]
    fn test_bom_does_not_prefix_first_header() {
        let doc = parse("\u{FEFF}term,language,gloss\nhello,en,greeting\n");
        assert_eq!(doc.sections.len(), 1);
        assert!(doc.sections[0].content.starts_with("term: hello"));
        assert!(!doc.sections[0].content.contains('\u{FEFF}'));
    }

    #[test]
    fn test_empty_row_skipped() {
        let doc = parse("term,gloss\nhello,greeting\n,\nciao,saluto\n");
        assert_eq!(doc.sections.len(), 2);
        assert_eq!(doc.sections[0].header, "hello");
        assert_eq!(doc.sections[1].header, "ciao");
    }

    #[test]
    fn test_header_only_yields_no_row_sections() {
        let doc = parse("term,language,gloss\n");
        assert_eq!(doc.sections.len(), 0);
        assert_eq!(doc.content, "");
        assert_eq!(doc.doc_type, "csv");
    }

    #[test]
    fn test_empty_first_cell_uses_row_n() {
        let doc = parse("id,note\n,orphan\n");
        assert_eq!(doc.sections.len(), 1);
        assert_eq!(doc.sections[0].header, "row 1");
        assert_eq!(doc.sections[0].content, "id: \nnote: orphan");
    }
}
