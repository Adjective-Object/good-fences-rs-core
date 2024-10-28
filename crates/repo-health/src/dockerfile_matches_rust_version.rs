use dockerfile_parser::{Dockerfile, RunInstruction, ShellOrExecExpr};
use std::collections::HashMap;

#[test]
fn test_dockerfile_rust_version_matches_root_toml() {
    let repo_root = repo_root::find_git_root();

    // get the version from rust-toolchain.toml
    let toolchain: HashMap<String, toml::Value> = toml::from_str(
        std::fs::read_to_string(repo_root.join("rust-toolchain.toml"))
            .unwrap()
            .as_str(),
    )
    .unwrap();
    println!("toolchain_toml: {:#?}", toolchain);
    let intended_channel = toolchain
        .get("toolchain")
        .unwrap()
        .get("channel")
        .unwrap()
        .as_str()
        .unwrap();
    let intended_profile = toolchain
        .get("toolchain")
        .unwrap()
        .get("profile")
        .map(|x| x.as_str().unwrap())
        .unwrap_or_else(|| "default");

    let mut intended_components = toolchain
        .get("toolchain")
        .unwrap()
        .get("components")
        .unwrap()
        .as_array()
        .unwrap()
        .iter()
        .map(|v| v.as_str().unwrap())
        .collect::<Vec<&str>>();
    intended_components.sort();

    let dockerfile_path = repo_root.join(".devcontainer/container/Dockerfile");
    let dockerfile_reader = std::fs::File::open(dockerfile_path).unwrap();
    let parsed_dockerfile: Dockerfile = Dockerfile::from_reader(&dockerfile_reader).unwrap();

    // find the part where we install with rustup
    let rustup_install: String = parsed_dockerfile
        .instructions
        .iter()
        .find_map(|instruction| {
            if let dockerfile_parser::Instruction::Run(RunInstruction {
                expr: ShellOrExecExpr::Shell(shell_expr),
                ..
            }) = instruction
            {
                let expr_string = format!("{}", shell_expr);
                if expr_string.contains("rustup toolchain install") {
                    Some(expr_string)
                } else {
                    None
                }
            } else {
                None
            }
        })
        .expect("rustup toolchain call should exist in dockerfile");
    println!("rustup_install: {}", rustup_install);

    // get rust version
    let rust_channel_re = regex::Regex::new(r#"rustup toolchain install ([^\s'"]+)"#).unwrap();
    let rust_channel = rust_channel_re
        .captures(&rustup_install)
        .expect("should be able to find toolchai nversion")
        .get(1)
        .unwrap()
        .as_str();

    if !rust_channel.starts_with(intended_channel) {
        panic!(
            "Dockerfile rust channel {} does not match rust-toolchain.toml channel {}",
            rust_channel, intended_channel
        );
    }

    let profile_re = regex::Regex::new(r#"--profile=([^\s'"]+)"#).unwrap();
    let profile = profile_re
        .captures(&rustup_install)
        .map(|cap| cap.get(1).unwrap().as_str())
        .unwrap_or("default");
    assert_eq!(profile, intended_profile);

    let component_re = regex::Regex::new(r#"--component=([^\s'"]+)"#).unwrap();
    let mut components = component_re
        .captures_iter(&rustup_install)
        .map(|cap| cap.get(1).unwrap().as_str())
        .collect::<Vec<&str>>();
    components.sort();
    assert_eq!(components, intended_components);
}
