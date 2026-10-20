use opentermius::{
    snippets::{self, Snippet},
    store::Store,
};

#[test]
fn snippet_create_update_and_delete_survive_store_reopen() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("opentermius-native.json");
    let mut store = Store::open(path.clone()).unwrap();
    let id = snippets::save(
        &mut store.data.snippets,
        Snippet {
            name: "Deploy staging".into(),
            script: "echo 你好\r\necho ready".into(),
            tags: vec!["staging".into()],
            ..Snippet::default()
        },
        100,
    )
    .unwrap();
    store.persist().unwrap();

    let mut store = Store::open(path.clone()).unwrap();
    let mut edited = snippets::find(&store.data.snippets, &id).unwrap().clone();
    assert_eq!(edited.script, "echo 你好\r\necho ready");
    assert_eq!(edited.tags, ["staging"]);
    edited.name = "Deploy production".into();
    edited.script = "echo updated".into();
    snippets::save(&mut store.data.snippets, edited, 200).unwrap();
    store.persist().unwrap();

    let mut store = Store::open(path.clone()).unwrap();
    let matches = snippets::search(&store.data.snippets, "PRODUCTION");
    assert_eq!(matches.len(), 1);
    assert_eq!(matches[0].id, id);
    assert_eq!(matches[0].script, "echo updated");
    assert_eq!(matches[0].created_at, Some(100));
    assert_eq!(matches[0].updated_at, Some(200));
    assert!(snippets::delete(&mut store.data.snippets, &id));
    store.persist().unwrap();
    assert!(Store::open(path).unwrap().data.snippets.is_empty());
}
