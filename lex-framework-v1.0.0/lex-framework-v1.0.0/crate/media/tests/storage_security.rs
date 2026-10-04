use media::{FileSystemStorage, MediaStorage};

#[tokio::test]
async fn concurrent_uploads_with_the_same_stem_keep_their_own_contents() {
    let directory = tempfile::tempdir().unwrap();
    let storage = FileSystemStorage::new(directory.path()).await.unwrap();
    let png = vec![b'p'; 1024 * 1024];
    let jpg = vec![b'j'; 1024 * 1024];
    let (a, b) = tokio::join!(
        storage.put("avatar.png", &png),
        storage.put("avatar.jpg", &jpg)
    );
    a.unwrap();
    b.unwrap();
    assert_eq!(storage.get("avatar.png").await.unwrap(), png);
    assert_eq!(storage.get("avatar.jpg").await.unwrap(), jpg);
}

#[tokio::test]
async fn an_upload_cannot_overwrite_an_existing_object_named_like_a_temporary_file() {
    let directory = tempfile::tempdir().unwrap();
    let storage = FileSystemStorage::new(directory.path()).await.unwrap();
    storage
        .put("avatar.lex-upload-tmp", b"existing")
        .await
        .unwrap();
    storage.put("avatar.png", b"new").await.unwrap();
    assert_eq!(
        storage.get("avatar.lex-upload-tmp").await.unwrap(),
        b"existing"
    );
}

#[cfg(unix)]
#[tokio::test]
async fn symlink_inside_media_root_cannot_escape_to_another_directory() {
    let directory = tempfile::tempdir().unwrap();
    let outside = tempfile::tempdir().unwrap();
    std::fs::write(outside.path().join("secret"), b"private").unwrap();
    std::os::unix::fs::symlink(outside.path(), directory.path().join("escape")).unwrap();
    let storage = FileSystemStorage::new(directory.path()).await.unwrap();
    assert!(storage.get("escape/secret").await.is_err());
    assert!(storage.put("escape/new", b"bad").await.is_err());
    assert!(storage.delete("escape/secret").await.is_err());
    assert_eq!(
        std::fs::read(outside.path().join("secret")).unwrap(),
        b"private"
    );
    assert!(!outside.path().join("new").exists());
}

#[tokio::test]
async fn malformed_keys_are_rejected_and_missing_deletes_are_idempotent() {
    let directory = tempfile::tempdir().unwrap();
    let storage = FileSystemStorage::new(directory.path()).await.unwrap();
    for key in ["", "../outside", "/absolute", "a\\b", "a/./b", ".", "a//b"] {
        assert!(storage.put(key, b"bad").await.is_err(), "accepted {key:?}");
    }
    storage.delete("missing").await.unwrap();
    assert!(storage.get("missing").await.is_err());
}
