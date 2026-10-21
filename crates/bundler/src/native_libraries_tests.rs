use super::*;

#[test]
fn libraries_jaccard_ranks_against_current_union_with_stable_ties() {
    let masks = [
        ["a", "b"].as_slice(),
        ["a", "b", "c"].as_slice(),
        ["c"].as_slice(),
        ["a"].as_slice(),
        ["unrelated"].as_slice(),
    ];
    let inputs: Vec<_> = masks
        .iter()
        .map(|mask| {
            (
                mask.iter().map(|s| RootId::Entry((*s).into())).collect(),
                10,
            )
        })
        .collect();
    let merges = select(&inputs, 40, Vec::new(), |state, members, next| {
        let mut state = state.clone();
        state.push((*members.first().unwrap(), next));
        Ok(Some(state))
    })
    .unwrap();
    // First pick 2/3 rather than 1/2; after expanding the union, c ties a.
    assert_eq!(merges, [(0, 1), (0, 2), (0, 3)]);
}
