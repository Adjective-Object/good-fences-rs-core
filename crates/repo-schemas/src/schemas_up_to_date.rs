use path_slash::PathBufExt;
use schemars::schema_for;
use std::path::{Path, PathBuf};
use unused_finder::UnusedFinderJSONConfig;

fn enforce_schema(file_path: &Path, schema: &schemars::schema::RootSchema) {
    // Check if the file exists
    if !file_path.exists() {
        // Just write the intended schema
        let schema_str = serde_json::to_string_pretty(schema).unwrap();
        std::fs::create_dir_all(file_path.parent().unwrap()).unwrap();
        std::fs::write(file_path, schema_str).unwrap();
    } else {
        // read the schmea from the file
        let file_str = std::fs::read_to_string(file_path).unwrap();
        let file_schema: schemars::schema::RootSchema = serde_json::from_str(&file_str).unwrap();

        if file_schema != *schema {
            // overwrite with the intended schema if the original is out of date
            let schema_str = serde_json::to_string_pretty(schema).unwrap();
            std::fs::write(file_path, schema_str).unwrap();

            // fail the test with a pretty_eq comparison
            panic!(
            "schema mismatch for {:?}. The schema file has been updated with the intended comments, which you should commit",
            file_path
        );
        }
    }
}

#[test]
fn test_unused_config_schema() {
    let repo_root = repo_root::find_git_root();
    let schemadir_path = repo_root.join(PathBuf::from_slash("schemas/"));

    // Check unused-config.schema.json
    enforce_schema(
        &schemadir_path.join("unused-config.schema.json"),
        &schema_for!(UnusedFinderJSONConfig),
    );
}
