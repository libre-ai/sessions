//! §3 observability SLO: the grounded-question pipeline (retrieve + generate +
//! verify) p99 under 5 s against a real model. Gated; run with DATABASE_URL +
//! AI_* env (e.g. a fast local model). Measures the real pipeline — short JSON
//! questions, not an arbitrary 200-token completion.

use std::time::Instant;

use presto_rag::corpus::{CorpusStore, RetrievalScope};
use presto_rag::pipeline::grounded_question;
use presto_rag::provider::OpenAiCompatible;

#[tokio::test]
#[ignore = "requires DATABASE_URL + AI_BASE_URL (real model); §3 generation SLO"]
async fn generation_pipeline_p99_under_5s() {
    let (Ok(db), Ok(provider)) = (std::env::var("DATABASE_URL"), OpenAiCompatible::from_env())
    else {
        eprintln!("skipping: set DATABASE_URL + AI_BASE_URL to run");
        return;
    };
    let corpus = CorpusStore::connect(&db).await.expect("connect");
    corpus
        .ingest(
            "default",
            0,
            "sun",
            "The Sun is the star at the center of the Solar System, about 1.39 million \
             kilometres in diameter.\n\nMercury is the smallest planet and the closest to \
             the Sun.\n\nThe Sun's core reaches about 15 million degrees Celsius.",
            &provider,
        )
        .await
        .expect("ingest");
    let scope = RetrievalScope::wedge();

    // Warm-up: model load + first-token latency is not part of steady-state SLO.
    let _ = grounded_question(&scope, "the Sun", &corpus, &provider).await;

    let n = 12usize;
    let mut samples = Vec::with_capacity(n);
    for _ in 0..n {
        let t = Instant::now();
        let q = grounded_question(&scope, "the Sun", &corpus, &provider).await;
        let ms = t.elapsed().as_millis();
        assert!(q.is_some(), "the pipeline must produce a grounded question");
        samples.push(ms);
    }
    samples.sort_unstable();
    let p50 = samples[n / 2];
    let p99 = samples[(((n - 1) as f64) * 0.99).round() as usize];
    eprintln!(
        "generation pipeline (retrieve+generate+verify): p50={p50}ms p99={p99}ms over {n} runs"
    );

    assert!(
        p99 < 5000,
        "generation pipeline p99 {p99}ms must be < 5000ms"
    );
}
