use path_slash::{self, PathBufExt};
use pretty_assertions::assert_eq;
use std::{
    fs,
    path::{Path, PathBuf},
};

fn read_raw_json(path: impl AsRef<Path>) -> serde_hjson::Value {
    let content = fs::read_to_string(path).unwrap();
    serde_hjson::from_str(&content).unwrap()
}

fn apply_single_json_value_patch(
    base: &mut serde_hjson::Value,
    path: &str,
    patch_value: serde_hjson::Value,
) {
    let segments = path.split('.').collect::<Vec<&str>>();
    if segments.is_empty() {
        return;
    }

    let mut current = base;
    // traverse to last segment
    for segment in segments.iter().take(segments.len() - 1) {
        current = current
            .as_object_mut()
            .unwrap()
            .entry(segment.to_string())
            .or_insert(serde_hjson::Value::Object(Default::default()));
    }
    // assign last value
    let last_seg = segments.last().unwrap();
    match current.as_object_mut().unwrap().get_mut(*last_seg) {
        Some(value) => {
            *value = patch_value;
        }
        _ => {
            // If inserting new value, insert at the end of the file
            current
                .as_object_mut()
                .unwrap()
                .insert(last_seg.to_string(), patch_value);
        }
    };
}

fn apply_json_patch(base: &mut serde_hjson::Value, patch: serde_hjson::Value) {
    patch.as_object().unwrap().iter().for_each(|(key, value)| {
        apply_single_json_value_patch(base, key, value.clone());
    });
}

#[test]
fn test_podman_devcontainer_matches_docker_devcontainer() {
    let repo_root = repo_root::find_git_root();
    // read both devcontainer.json files
    let default_path = repo_root.join(PathBuf::from_slash(
        ".devcontainer/default/devcontainer.json",
    ));
    let mut expected_devcontainer = read_raw_json(&default_path);

    let podman_path = repo_root.join(PathBuf::from_slash(
        ".devcontainer/podman/devcontainer.json",
    ));
    let podman_devcontainer = read_raw_json(&podman_path);

    let patch_path = repo_root.join(PathBuf::from_slash(".devcontainer/podman/patch.json"));
    let patch_json = read_raw_json(&patch_path);

    // Get expected podman dockerfile by applying the podman patch to the base devcontainer
    apply_json_patch(&mut expected_devcontainer, patch_json.clone());

    // compare the two values
    assert_eq!(expected_devcontainer, podman_devcontainer);
}
