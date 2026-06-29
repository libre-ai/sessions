//! Flashcard deck persistence over Postgres (Prod2). Requires `DATABASE_URL`;
//! ignored by default. Proves a generated deck survives the session and is
//! retrievable, round-tripping its SM-2 state unchanged.

use presto_core::protocol::Flashcard;
use presto_server::flashcard_store::{FlashcardStore, PostgresFlashcardStore};

#[tokio::test]
#[ignore = "requires DATABASE_URL (Postgres); run in the integration job"]
async fn deck_persists_and_retrieves_over_postgres() {
    let Ok(url) = std::env::var("DATABASE_URL") else {
        eprintln!("skipping: set DATABASE_URL to run");
        return;
    };

    let store = PostgresFlashcardStore::connect(&url)
        .await
        .expect("connect");
    let owner = format!("owner-{}", std::process::id());

    let deck = vec![
        Flashcard {
            section_id: "doc#p0".into(),
            front: "What is mitochondria?".into(),
            back: "The powerhouse of the cell.".into(),
            ease_factor: 2.5,
            interval_days: 0,
        },
        Flashcard {
            section_id: "doc#p2".into(),
            front: "What does Rust guarantee?".into(),
            back: "Memory safety without a GC.".into(),
            ease_factor: 2.6,
            interval_days: 3,
        },
    ];

    store.save_deck(&owner, &deck).await.expect("save");
    let loaded = store.load_deck(&owner).await.expect("load");
    assert_eq!(loaded, deck, "the persisted deck must round-trip unchanged");

    // Re-saving replaces, not appends.
    store.save_deck(&owner, &deck[..1]).await.expect("resave");
    let reloaded = store.load_deck(&owner).await.expect("reload");
    assert_eq!(reloaded.len(), 1, "save replaces the prior deck");

    eprintln!(
        "flashcard deck persisted + retrieved over Postgres ({} cards)",
        loaded.len()
    );
}
