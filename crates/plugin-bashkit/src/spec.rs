//! The definition of the Pack's one tool, an embedded JSON file.
//!
//! The file holds the name the model calls the tool by, the description
//! the model reads, and the JSON Schema its arguments must match. It
//! starts from Cursor's definition of its `Shell` tool, edited where a
//! line describes behavior this Pack does not have. `schemas/EDITS.md`
//! records every edit.

use serde::Deserialize;
use serde_json::Value;

use crate::shell::{OUTPUT_CAP, TIMEOUT_MS};

/// The tool's definition, as the Pack hands it to the Host.
#[derive(Debug)]
pub(crate) struct Definition {
    /// The name the model calls the tool by.
    pub(crate) wire_name: String,
    /// The description the model reads.
    pub(crate) description: String,
    /// The JSON text of the JSON-Schema `object` the tool's arguments
    /// must match.
    pub(crate) parameters: String,
}

/// The tool's file, as the file itself holds it.
#[derive(Debug, Deserialize)]
struct File {
    /// The name the model calls the tool by.
    wire_name: String,
    /// The description the model reads.
    description: String,
    /// The JSON-Schema `object` the tool's arguments must match.
    parameters: Value,
}

/// The source of the tool's file.
const SOURCE: &str = include_str!("../schemas/shell.json");

/// The definition, parsed from the embedded file. The Plugin calls this
/// once, when it loads, and keeps the result in its instance.
///
/// # Errors
/// Returns why the file does not parse. The file is embedded, so this can
/// only happen after an edit.
pub(crate) fn load() -> Result<Definition, serde_json::Error> {
    let file: File = serde_json::from_str(&fill(SOURCE))?;
    Ok(Definition {
        wire_name: file.wire_name,
        description: file.description,
        parameters: file.parameters.to_string(),
    })
}

/// `source` with its `{...}` placeholders filled from the constants the
/// code uses, so the text the model reads and the limit the code enforces
/// cannot drift apart.
fn fill(source: &str) -> String {
    source
        .replace("{output_cap}", &OUTPUT_CAP.to_string())
        .replace("{timeout_ms}", &TIMEOUT_MS.to_string())
}

#[cfg(test)]
mod tests {
    use serde_json::{Value, json};

    use super::{SOURCE, load};

    #[test]
    fn the_definition_parses_and_names_the_tool_shell() {
        let definition = load().expect("the embedded definition parses");
        assert_eq!(definition.wire_name, "Shell");
        assert!(
            definition
                .description
                .starts_with("Executes a given command in a shell session")
        );
    }

    #[test]
    fn every_placeholder_is_filled_from_the_constant_the_code_enforces() {
        assert!(SOURCE.contains("{output_cap}") && SOURCE.contains("{timeout_ms}"));
        let definition = load().expect("the embedded definition parses");
        assert!(
            definition
                .description
                .contains("Output past 20000 characters")
        );
        assert!(definition.parameters.contains("Defaults to 30000ms"));
        for text in [&definition.description, &definition.parameters] {
            assert!(
                !text.contains("{output_cap}") && !text.contains("{timeout_ms}"),
                "a placeholder is left in {text}"
            );
        }
    }

    #[test]
    fn the_schema_declares_the_four_parameters_and_requires_the_command() {
        let definition = load().expect("the embedded definition parses");
        let schema: Value =
            serde_json::from_str(&definition.parameters).expect("the schema is JSON");
        assert_eq!(schema["type"], "object");
        assert_eq!(schema["required"], json!(["command"]));
        let mut names: Vec<&str> = schema["properties"]
            .as_object()
            .expect("the properties are an object")
            .keys()
            .map(String::as_str)
            .collect();
        names.sort_unstable();
        assert_eq!(
            names,
            [
                "block_until_ms",
                "command",
                "description",
                "working_directory"
            ]
        );
        assert_eq!(schema["properties"]["block_until_ms"]["type"], "integer");
        assert_eq!(schema["properties"]["block_until_ms"]["minimum"], 1);
    }
}
