# Vector Embedding Sourcing for jinn + overgraph: Local CPU vs Cloud

*Research report — October 2026. Feeds the plan for how the overgraph MCP server (or jinn) generates embedding vectors. Every claim is bucket-classified inline; footnotes at the bottom.*

---

## Bottom line

1. **Local CPU embedding with a small model is a mainstream, proven pattern** — not a fringe move. The default Ollama embedding model (`nomic-embed-text`) has ~74–87M pulls[^ollama-nomic][^morphllm], fastembed exists precisely to make local CPU embedding one call in Rust[^fastembed-docs], and people run small embedding models **in the browser, in real time, on a phone**[^hn-localrag].
2. **Speed is a non-issue at jinn's scale.** ~1–2 ms per short-text embedding on a laptop CPU (fp32, ONNX)[^rust-embedding-bench]; 2.5 ms for an INT8-quantized E5-small query on a Graviton3 server core[^vespa]. Bulk indexing runs thousands of embeddings/sec on CPU alone[^rust-embedding-bench].
3. **Small models are measurably less accurate than frontier cloud embedders** — roughly 5–15 points on retrieval benchmarks — but the newest small locals close much of the gap, and practitioner consensus is that for agent-memory-scale corpora the difference is usually not the thing that kills you[^hn-localrag][^embedrs]. The embedder choice *does* swing outcomes more than most pipeline tweaks[^memdelta].
4. **Cloud embedding is cheap enough that price is not the deciding factor.** OpenAI text-embedding-3-small is $0.02/1M tokens[^openai-docs][^economize]; an agent writing 100M tokens of embedded memory a month costs **$2**[^spheron].
5. **The real decision for jinn is not "local vs cloud" — it's *who runs the embedder***: in-process in the server child, a user-configured OpenAI-compatible endpoint, or raw vectors through the tool. All three are evidenced patterns (see "What this means for jinn" below). One hard constraint from the overgraph source: **the dimension is locked into the DB the first time vector config lands**, so the model choice is a one-way door[^overgraph-source].

---

## Q1 — Do people actually run local CPU embedders, or shove it to the cloud?

**Both, split by use case.** Cloud APIs are the default in industry RAG; local CPU is the default in the personal/agent-memory/self-host world, and it is large.

### Forum / social (primary bucket for "what people actually do")

- r/ollama thread asking exactly this question ("which embedding models do you actually use"): answers mix **local** (Qwen3-embedding:8b, nomic-embed-text, EmbeddingGemma-300m — "really fast… I don't feel different from the bigger ones") and **cloud** (Voyage-3.5-Large "if using proprietary APIs")[^reddit-rag]. Nobody in the thread fine-tunes their own.
- HN "So you wanna build a local RAG" (390 points): a running demo embeds **in the browser** (~50MB weights + ONNX runtime downloaded client-side) and returns semantic results in real time, "even on a phone"[^hn-localrag]. One commenter runs "hybrid semantic/bm25 search on my MacBook Pro across millions of documents without too much trouble"[^hn-localrag]. Advice for CPU self-hosting: stay ≤100M parameters[^hn-localrag].
- A real-world MCP-server precedent that matches jinn's shape almost exactly: the **Nextcloud MCP server** embeds docs/notes itself with a choice of "ollama and openai" as embedding providers — i.e., local daemon or cloud behind one config knob[^hn-localrag-nextcloud].
- Counterweight (also from the same HN thread): several experienced practitioners argue embeddings are overkill for agentic search over code/text — iterative grep + a frontier LLM refines queries well, and Claude Code famously doesn't embed at all while Cursor does[^hn-localrag]. This is about *whether to vectorize at all* (overgraph decision, already made) rather than *where to embed*.

### Practitioner blogs

- A self-hosted agent-memory guide frames the split cleanly: local embedding endpoints exist for **privacy, offline, no rate limits, no deprecation risk** — and explicitly says "the non-reason is raw quality. Top hosted embedding models still edge out small local ones on benchmarks." The standard local pattern is an OpenAI-compatible daemon (Ollama on `:11434/v1`, LM Studio on `:1234/v1`)[^openclawai].
- A project ADR choosing `nomic-embed-text` via Ollama for a CPU-only offline laptop documents the reasoning and the tradeoff (quality ceiling below cloud; all-MiniLM "quality drops noticeably on technical content"; switching models forces a re-embed of the collection)[^localrag-adr].

### Academic

- MemDelta (preprint, not peer-reviewed) studies agent-memory evals and treats cloud embeddings as the default arm of its experiments; its finding relevant to us is that the embedding model alone swings retrieval accuracy by ~6pp — enough to flip a system-level conclusion — and MiniLM-based RAG trails cloud-embedding RAG by ~11pp on their benchmark[^memdelta].

### Vendor

- Nomic markets `nomic-embed-text` as beating OpenAI ada-002 and text-embedding-3-small on short/long context — vendor claim from Nomic's own eval, treat as marketing until independently reproduced[^ollama-nomic].

**Empty buckets:** no journalism and no conference talks found or needed on this question; the practitioner evidence is the load-bearing kind and it is abundant.

---

## Q2 — How fast is a CPU embedding with a small model?

**Benchmark / measurement sources:**

| Measurement | Number | Hardware / conditions | Source kind |
|---|---|---|---|
| Single-query embedding, all-MiniLM-L6-v2 (22M params, 384-dim), fp32 ONNX | **1.10 ms p50** (ort), **1.71 ms** (fastembed) | Apple M4 Max laptop CPU | Reproducible benchmark harness, outputs validated cosine ≥0.999 vs Python reference[^rust-embedding-bench] |
| Query embedding (8 words), E5-small-v2 **INT8** | **2.5 ms** | AWS Graviton3 CPU (c7g.2xlarge) | Vendor benchmark (Vespa — sells search infra; methodology and code public)[^vespa] |
| Batch throughput, MiniLM, b=32, short text | **~2,800–3,050 emb/s** CPU | M4 Max, 8 threads | [^rust-embedding-bench] |
| Batch throughput, GPU offload | **~9,700 emb/s** | llama-cpp-2 + Metal, M4 Max | [^rust-embedding-bench] |
| Same model via Ollama HTTP daemon | **11 ms/query** (vs 1–2 ms in-process) — the HTTP layer is most of the cost | M4 Max, F16 GGUF | [^rust-embedding-bench] |
| 103-word doc throughput, CPU | ~260–315 docs/s batched (~3–4 ms/doc) | M4 Max / Graviton3 | [^rust-embedding-bench][^vespa] |
| INT8 weight quantization | **2.7–3.4× faster**, retains **94–98%** quality — on CPU; a wash on Apple Silicon (x86 VNNI is where INT8 pays) | ONNX runtime | [^vespa][^rust-embedding-bench] |
| GPU vs CPU | T4 GPU only 4–7× faster than Graviton3 CPU | | [^vespa] |

**Reading:** for jinn's workload — one node or one query string at a time, short text — CPU cost is single-digit milliseconds, invisible next to an LLM round-trip. Even the slowest shape measured (HTTP to a local daemon) is ~11 ms. The practitioner "qwen3-embedding latency wasn't great for my usecase" comment refers to *0.6B–8B-class* models, not the 22–33M class[^hn-localrag]. If we ever want near-frontier quality locally, Qwen3-Embedding-0.6B (1024-dim, 32K context, Apache-2.0)[^qwen3-paper] is the candidate — several times slower than MiniLM-class but still CPU-viable per community use[^reddit-rag].

**Population caveat (read the setup):** all numbers are short-text (query/paragraph) regime on Apple Silicon or ARM server cores. A 512-token context limit on bge-small/MiniLM also caps what one embedding can cover — fine for node payloads, relevant if a user stuffs whole documents into nodes.

---

## Q3 — Is a small model accurate enough?

**Depends on the bar. The numbers:**

- MTEB(eng, v2) leaderboard: **bge-small-en-v1.5** (33M params) mean **64.30**; **Qwen3-Embedding-0.6B** mean **70.47** (retrieval 61.83); bge-large-en-v1.5 (335M) 65.89[^mteb]. Note how far a 0.6B local model has climbed: it sits **above** the 335M previous-gen giant.
- OpenAI's self-reported evals: text-embedding-3-small 62.3, 3-large 64.6 (their own 2024 MTEB config — *not directly comparable* to leaderboard rows; different eval versions)[^openai-docs][^cognee].
- Legacy MTEB v1, directly comparable table from the model card: bge-small retrieval **51.68** vs ada-002 **49.25** (and bge-large 53.9)[^bge-card] — i.e., the 33M model already beat the previous cloud default at release.
- An 8-dimension independent benchmark: MiniLM-L6 Spearman ρ **0.81** vs OpenAI **0.91**, Voyage 0.89, Gemini 0.94 — a real gap, while MiniLM wins cluster separation and costs nothing[^embedrs]. (Practitioner benchmark; the same authors pick MiniLM as "the only model small enough for app embedding.")
- MemDelta's controlled experiment: swapping only the embedding model moved retrieval-adjacent accuracy **+6.2pp**, and MiniLM-RAG landed ~11pp under cloud-RAG — the embedder is not a rounding error[^memdelta].
- Vespa: hybrid retrieval (vectors + BM25/lexical) buys **+3–5pp nDCG@10 over semantic-only, for every model tested**[^vespa] — directly actionable, since overgraph ships sparse vectors + fusion modes; a small dense model *plus* sparse keywords recovers a chunk of the cloud gap architecturally.

**Practitioner verdict:** "as long as you're happy with the model and always use the same one, there's not a ton of difference from one to the next" for personal-corpus retrieval[^reddit-rag]; the failure mode that matters is eval-vs-real-users mismatch (devs test with terms they know; users search with different words), which no model choice fixes[^hn-localrag].

**Honest summary:** small local models give up ~5–15pp vs frontier cloud on retrieval benchmarks; newest-generation small locals (Qwen3-0.6B, EmbeddingGemma-300m) shrink that materially; for agent memory over thousands of notes, practitioners report the gap rarely decides anything. If a user's graph *is* their memory and quality is load-bearing, pointing at a cloud embedder costs ~$2/month — quality is purchasable trivially, which is why "who runs the embedder" should stay configurable rather than baked in.

---

## Q4 — Cloud pricing: does it matter?

**No, at jinn's scale.** Per-1M-token input prices, from provider/pricing pages (vendor bucket, current as of late 2026):

| Provider / model | Price per 1M tokens | Notes |
|---|---|---|
| OpenAI text-embedding-3-small | **$0.02** (Batch API: $0.01)[^economize][^costgoat] | 1536-dim (Matryoshka-truncatable), 8191-token input[^openai-docs] |
| OpenAI text-embedding-3-large | $0.13[^pricepertoken] | 3072-dim |
| Cohere Embed 5 Fast / Pro | $0.08 / $0.12[^cohere] | 128K context, multimodal, shared embedding space |
| Voyage voyage-3 / 3-lite | $0.06 / $0.02[^spheron] | |
| Local (CPU, small model) | **$0** after a one-time ~23–130MB model download | [^fastembed-docs][^embedrs] |

Volume context: 100M tokens/month through the cheapest OpenAI model = **$2/month**[^spheron]. A self-hosted GPU beats that only around 50–100M tokens/month (A100 math — vendor-motivated source, and irrelevant for a desktop app)[^spheron]. Practitioner dev-loop framing: local embedding costs nothing while debugging and never rate-limits[^ruslanmv].

---

## Overgraph-side constraint (verified in source, primary)

From the `overgraph` 0.17.1 crate source (fetched from crates.io):

- `DbOptions.dense_vector: Option<DenseVectorConfig>` defaults to `None`. `DenseVectorConfig { dimension, metric, hnsw }` is persisted into the DB manifest at open time[^overgraph-source].
- `reconcile_dense_vector_manifest`: opening a DB whose manifest has **no** vector config with a **requested** config is allowed and **upgrades the DB in place**; opening with a config **mismatching** a persisted one is a hard error; `None→None` and `Some→Some(matching)` are fine[^overgraph-source].
- Consequences: (a) shipping graph-only now and adding vectors later is migration-free, as previously established; (b) **the first write carrying vector config permanently fixes dimension + metric for that DB** — so jinn's config surface (which model/embedder) must be decided before a DB's first vector, and changing it later means re-embedding every node by rewriting them (overgraph has no re-embed API).

---

## What this means for jinn's integration shape (evidence, not a decision)

Three viable patterns, all with precedent; listing what the evidence says about each:

1. **In-process local model in the overgraph server child** (fastembed/ort, default bge-small-en-v1.5 or EmbeddingGemma-300m): 1–2 ms/embed on CPU, fully offline, zero ongoing cost, no user setup beyond a first-run model download. Costs: `ort` is a heavy native dependency inside the jinn binary (build complexity on Windows/macOS/Linux), ~25–130MB downloaded weights, and the model choice is frozen per DB by the dimension lock. fastembed is sync/no-tokio — fits the child-process shape fine[^fastembed-docs][^rust-embedding-bench].
2. **User-configured OpenAI-compatible endpoint** (`base_url` + `model` + optional `api_key`): one HTTP client, no new native deps; the user points it at Ollama (local, free), LM Studio, or a cloud provider (trivial cost) — this is exactly the Nextcloud-MCP pattern[^hn-localrag-nextcloud] and the standard self-host pattern[^openclawai]. Quality scales with whatever the user aims it at; Ollama HTTP overhead is ~10ms/embed — irrelevant here[^rust-embedding-bench]. No model ships by default: vector search needs the user to have *some* embedder.
3. **Raw vectors through the tool** (agent/user supplies arrays): zero deps, zero setup, works today; quality is entirely the caller's problem; clunky for an agent (it can't call an embeddings API itself without a tool for that).

The buckets do not disagree on the physics (CPU latency, accuracy gap, pricing); they disagree only on emphasis — forum evidence loves local because its participants self-host; vendor evidence (Vespa, Spheron, Cohere) frames quality-at-scale, which is not jinn's problem.

---

## Gaps (what I did not find)

- **No benchmark isolates "agent-memory retrieval quality, small local vs cloud, on personal-corpus sizes"** — MemDelta is the closest (benchmark-corpus scale, agent-memory framing) and it's a preprint. The practitioner claim "you won't notice at hobby scale" is consensus testimony, not measurement.
- **No published numbers for fastembed's bge-small on Windows x86 consumer CPUs** (Vespa = ARM server; rust-embedding-bench = Apple Silicon; both authors flag x86-VNNI INT8 as likely faster).
- **No independent reproduction of Nomic's "beats OpenAI 3-small" claim** — vendor figure only.
- **No data on EmbeddingGemma-300m beyond leaderboard/vendor entries and one Reddit testimonial** — promising (MRL dims 768/512/256/128 would ease the dimension lock) but thin evidence.

## Rabbit holes

- [Search Sensei — in-browser semantic vs BM25 demo](https://search-sensei.s3-website-us-east-1.amazonaws.com/) — embeds 1,000 news articles live in your browser via a 50MB WASM download; the existence proof that tiny-model CPU embedding is real-time.
- [FastTextEmbed](https://github.com/cemsina/fasttextembed) — a single-model, pure-C bge-small engine claiming 2.3× fastembed throughput at 90MB RAM; someone specialized the whole forward pass for one model and won. A sign of how hot this niche is, not a jinn option.
- [MemDelta's finding that Mem0 ties plain cloud RAG at 50× the write-path cost](https://arxiv.org/pdf/2606.29914) — the fancier agent-memory layer didn't beat good retrieval; relevant whenever overgraph's vector search gets compared to jinn's built-in session search.
- [Claude Code doesn't embed, Cursor does](https://cursor.com/docs/context/codebase-indexing) — the same product category, opposite retrieval strategies, both shipping fine.

---

## Footnotes

[^ollama-nomic]: [nomic-embed-text — Ollama library (86.7M downloads; vendor description)](https://ollama.com/library/nomic-embed-text:latest)
[^morphllm]: [Best Ollama Embedding Models 2026 — Morphllm (practitioner/aggregator; nomic 73.8M pulls)](https://www.morphllm.com/ollama-embedding-models)
[^fastembed-docs]: [fastembed — docs.rs (documentation/reference; local ONNX, sync, default BGE-small-en-v1.5)](https://docs.rs/fastembed/5.17.3/fastembed/)
[^hn-localrag]: [So you wanna build a local RAG? — Hacker News thread, 390 pts (forum; incl. in-browser embedding demo, ≤100M-param CPU advice, MacBook hybrid-search comment, agentic-grep counterargument)](https://news.ycombinator.com/item?id=46080364)
[^hn-localrag-nextcloud]: Same thread as above; comment by the Nextcloud MCP server author describing ollama/openai embedding providers. Separately cited here because it is a distinct, load-bearing precedent.
[^rust-embedding-bench]: [rust-embedding-bench — GitHub (benchmark/practitioner; M4 Max, all-MiniLM-L6-v2, backends validated vs Python reference)](https://github.com/jerrythomas/rust-embedding-bench)
[^vespa]: [Embedding Tradeoffs, Quantified — Vespa Blog, Jan 2026 (vendor benchmark; Graviton3/T4 hardware; code open-sourced in pyvespa)](https://blog.vespa.ai/embedding-tradeoffs-quantified/)
[^memdelta]: [MemDelta: Controlled Baselines and Hidden Confounds in Agent Memory Evaluation — arXiv preprint (academic, non-peer-reviewed)](https://arxiv.org/pdf/2606.29914)
[^openai-docs]: [Vector embeddings — OpenAI API docs (vendor docs; pricing-by-pages table, MTEB self-evals, Matryoshka dims)](https://developers.openai.com/api/docs/guides/embeddings)
[^economize]: [OpenAI text-embedding-3-small Pricing — Economize (pricing tracker; verified against OpenAI's page 2026-10)](https://www.economize.cloud/resources/open-ai/pricing/text-embedding-3-small/)
[^costgoat]: [OpenAI Embeddings API Pricing Calculator — CostGoat (pricing tracker; batch-tier pricing)](https://costgoat.com/pricing/openai-embeddings)
[^pricepertoken]: [OpenAI Embedding Models — Price Per Token (pricing tracker)](https://pricepertoken.com/embedding/author/openai)
[^cohere]: [Embed 5: Frontier Embedding Models for Enterprise — Cohere blog, Sep 2026 (vendor; includes third-party ViDoRe comparisons)](https://cohere.com/blog/embed-5)
[^spheron]: [Self-Host Embeddings and Rerankers — Spheron blog (vendor — sells GPU cloud; API price table + self-host crossover math)](https://www.spheron.network/blog/self-host-embedding-reranker-tei-gpu-cloud)
[^mteb]: [MTEB(eng, v2) leaderboard (benchmark leaderboard; model rows cited: bge-small-en-v1.5, Qwen3-Embedding-0.6B, bge-large)](https://leaderboard.mteb.org/benchmark/MTEB%28eng%2C%20v2%29?task_type=Retrieval)
[^cognee]: [Best Embedding Models in 2026 — Cognee (practitioner/vendor; notes which MTEB numbers are cross-comparable)](https://www.cognee.ai/best-embedding-models)
[^bge-card]: [BAAI/bge-small-en-v1.5 — Hugging Face model card (primary model documentation; legacy MTEB v1 table incl. ada-002)](https://huggingface.co/BAAI/bge-small-en-v1.5)
[^qwen3-paper]: [Qwen3 Embedding — arXiv (vendor tech report; 0.6B spec: 1024-dim, 32K ctx, MTEB results)](https://arxiv.org/html/2506.05176v1)
[^embedrs]: [embedrs / rust-embeddings — docs.rs + GitHub (practitioner benchmark; 8-model comparison table incl. ρ, robustness, cluster separation, cost)](https://github.com/goliajp/rust-embeddings)
[^openclawai]: [Local Embeddings for AI Agent Memory — OpenClawAI guide (practitioner/vendor — sells an agent product; OpenAI-compatible local endpoint pattern)](https://openclawai.io/blog/local-embeddings-ai-agent-memory-self-hosted)
[^localrag-adr]: [ADR 002: Default Embedding Model — n0nuser/LocalRAG (practitioner; CPU-only offline laptop constraints, re-embed-on-switch)](https://github.com/n0nuser/LocalRAG/blob/main/docs/adr/002-embedding-model-nomic-embed-text.md)
[^reddit-rag]: [RAG. Embedding model. What do u prefer? — r/ollama (forum; what people actually run)](https://www.reddit.com/r/ollama/comments/1otsq4j/rag_embedding_model_what_do_u_prefer)
[^ruslanmv]: [Ollama: Run LLMs Locally — ruslanmv.com blog (practitioner; dev-loop cost framing)](https://ruslanmv.com/Best-of-the-Best/blog/posts/2025-12-27-package-ollama.html)
[^overgraph-source]: overgraph 0.17.1 crate source, fetched from static.crates.io — `src/engine/mod.rs::reconcile_dense_vector_manifest`, `src/types.rs::DenseVectorConfig` (primary artifact; unrefetchable-by-URL, kept locally at `/tmp/overgraph-0.17.1`)
[^checkapicost]: [OpenAI vs Cohere Embed cost comparison — checkapicost.com (affiliate/comparison site; used only for its TCO framing, not its conclusions)](https://checkapicost.com/vs/openai-embeddings-vs-cohere-embed)
