# `stealth-bunny-alpha` / Space Bunny Alpha — research findings

**Research date:** 2026-09-24  
**Angle:** Investigative/informational

## Executive finding

The model you are probably referring to is **Space Bunny Alpha**, with the canonical OpenRouter model ID **`stealth/space-bunny-alpha`**. I found no first-party evidence for an exact model ID `stealth-bunny-alpha`; the exact-slug catalog search returned no match, while the model page and public models API both confirm `stealth/space-bunny-alpha`.[^1][^2]

Space Bunny Alpha is a real, currently listed OpenRouter model entry, but it is an **anonymous third-party preview**, not a publicly identified foundation model. OpenRouter explicitly says it routes requests to the model and is not its developer, owner, or provider. The provider has chosen to remain anonymous during the preview.[^2]

## Confirmed technical facts

| Item | What OpenRouter currently reports |
|---|---|
| Model ID | `stealth/space-bunny-alpha` |
| Display name | Space Bunny Alpha |
| Listed release | September 23, 2026 |
| Context | 1,000,000 tokens |
| Maximum output | 524,288 tokens |
| Modalities | Text, image, and video input; text output |
| Price | $0 per million input tokens and $0 per million output tokens |
| Reasoning | Supported; advertised effort levels: low, medium, high, xhigh, max. The public API says reasoning is mandatory and defaults to `max`. |
| Tools | Function/tool calling supported |
| Structured output | `response_format` supported, but OpenRouter says it does not enforce JSON schemas |
| Tokenizer | Listed as `Other` / unspecified |
| Knowledge cutoff | Not disclosed (`null` in API metadata) |
| Parameter count | Not disclosed |
| Public weights | No Hugging Face identifier is provided in OpenRouter's model metadata; no public weights were located in this search |
| Provider | Anonymous; one hosting endpoint is labeled `Stealth` |

The strongest technical source is OpenRouter's model-specific `llms.txt`, which confirms the model ID, supported request parameters, API endpoints, and error behavior.[^3] The live public models API independently confirms the 1M context, 524,288 output ceiling, multimodal input, free pricing, tool parameters, and mandatory-reasoning metadata.[^2]

### What “stealth” means here

OpenRouter's own historical explanation of the program is that these are **prerelease models offered anonymously for a limited period**, with users able to test them before the formal public release and provide feedback to the model lab.[^4][^5] That is the evidence-based interpretation of the label: **anonymous preview / unreleased identity**, not merely a marketing adjective.

The Stealth Program EULA says:

- providers may offer models anonymously and free of charge for a limited time;
- OpenRouter may not disclose a provider's name or origin on request;
- models may be removed at any time;
- OpenRouter does not develop the models; they are developed or licensed by the applicable provider.[^6]

So the model is probably not a publicly documented model from OpenAI, Anthropic, Google, or another named lab. The identity question is genuinely unresolved.

## Availability and access

The model appears accessible through the OpenRouter API using:

```text
POST https://openrouter.ai/api/v1/chat/completions
model: stealth/space-bunny-alpha
```

OpenRouter's model page currently shows a hosted provider, recent uptime/availability telemetry, and an API playground. The provider page also lists it as the active Stealth model.[^2][^7]

However, availability is explicitly **temporary and not guaranteed**. The Stealth EULA permits OpenRouter or the provider to remove or suspend a model at any time, with or without notice.[^6] The current listing's zero price should therefore be treated as a preview arrangement, not evidence of permanent free inference.

## Performance evidence

The model page advertises “blazing-fast inference,” but that is provider/model metadata rather than an independent quality result. Current OpenRouter telemetry on the fetched model page reported approximately **83 tokens/second**, **1.70 seconds P50 latency**, **99.70% three-day uptime**, and **98.09% three-day availability**.[^2]

Those numbers are useful operational signals, but they are not a benchmark. They describe the current hosted route, not the underlying model across different hardware or workloads. The endpoint API also reports no current latency or throughput values in its endpoint record, which is another reason not to treat the speed claim as a durable model characteristic.[^3]

### Benchmark status

I found **no credible independent benchmark result** for Space Bunny Alpha. The Benchable model page lists its capabilities and endpoint but shows no benchmark executions or results.[^8] A contemporaneous OrcaRouter analysis likewise says that no independent evaluation or public benchmark run had been published at the time it investigated the launch, while emphasizing that all listed specifications are operator-supplied and unaudited.[^9]

Therefore, claims such as “strong coding capabilities” should be treated as a **listing description**, not as an established coding benchmark result. No verified parameter count, architecture, tokenizer family, training-data provenance, context-quality test, or long-output coherence test is publicly available in the sources reviewed.

## Data handling and privacy: an important discrepancy

The model-specific page says prompts and completions **may be retained by the provider but are not used for training**.[^2] The current model-specific EULA language supports the general possibility of provider retention and makes training use depend on what each model listing discloses.[^6]

However, OpenRouter's provider-level logging table lists the **Stealth** provider as “Prompts are retained for unknown period” and “May train.”[^10] This is a meaningful conflict in presentation:

- **Model-specific claim:** Space Bunny Alpha is not used for training.
- **Provider-level generic table:** Stealth may train, and retention duration is unknown.
- **Program EULA:** the listing is supposed to disclose whether a particular model is used for training.

The safest operational conclusion is: **do not treat this model as a zero-retention endpoint, and do not submit proprietary code, customer data, credentials, personal data, or regulated material until the provider's exact retention and training terms are clarified.** The model's own page does at least explicitly say it is not used for training, but the broader Stealth provider table makes the privacy posture less reassuring.

## Identity investigation

### What is confirmed

- The provider is deliberately anonymous.
- OpenRouter says it is only a routing layer.
- The model is hosted through a provider labeled Stealth.
- No Hugging Face model ID, public weights URL, vendor name, parameter count, or named architecture is exposed in the API metadata.

### What is not confirmed

No reliable source found in this investigation identifies:

- the company or laboratory behind the model;
- whether it is based on an existing open-weight model;
- the model's parameter count or architecture;
- the tokenizer family;
- the training-data mix or knowledge cutoff;
- a published license;
- a permanent post-preview price;
- an independent benchmark result.

A secondary article makes a reasonable observation that the “Bunny” codename itself provides no reliable clue about the vendor and that previous stealth releases have eventually been revealed under official names.[^9] That pattern makes a future reveal plausible, but it is not evidence about the current model's identity.

## Timeline

- **September 23, 2026:** Space Bunny Alpha was listed/released on OpenRouter. The public model API records its creation timestamp as September 23, 2026 at approximately 14:48 UTC.[^2]
- **September 23, 2026:** Early secondary coverage described it as a free anonymous preview with 1M context and multimodal support.[^9][^11]
- **September 24, 2026:** It remained publicly listed and served by the Stealth provider when checked. The exact user-provided slug `stealth-bunny-alpha` was not the canonical ID; the correct ID is `stealth/space-bunny-alpha`.[^1][^2]

## Context: earlier stealth models

OpenRouter's earlier announcements described Quasar Alpha as a prerelease, 1M-context, coding-oriented foundation model from an unnamed model lab, and Optimus Alpha as a general-purpose, coding-oriented 1M-context model available free during a stealth period. OpenRouter said users could help the lab refine performance through Discord feedback.[^4][^5]

The pattern shows that “stealth” has historically referred to an unreleased or limited-time preview. It does **not**, by itself, prove that the model is unreleased: a provider could use the program for an existing model, a preview alias, or an evaluation deployment. The terms and current listing simply establish anonymity and temporary access, not the model's underlying training status.

## Bottom line

**Space Bunny Alpha is a currently accessible, free, anonymous OpenRouter preview with unusually large advertised limits and broad claimed capabilities. Its identity and real-world quality remain unknown.** The most defensible description is:

> A third-party, anonymously operated model exposed by OpenRouter as `stealth/space-bunny-alpha`, advertised as a fast, coding-oriented, text/image/video model with a 1M-token context window, 524K-token maximum output, tool calling, structured output, and adjustable reasoning.

The main practical risks are provider opacity, unknown long-term availability, unclear data-retention policy, absence of independent benchmarks, and the possibility that the free preview will later disappear or be repriced.

## Rabbit holes

- [OpenRouter Stealth Program EULA — privacy, retention, training, removal, and anonymity terms](https://openrouter.ai/terms/stealth) — the primary source for why an anonymous preview may disappear and how user content can be used.
- [OpenRouter provider logging table](https://openrouter.ai/docs/features/privacy-and-logging) — the Stealth row conflicts with the model-specific “not used for training” statement and deserves a direct provider clarification.
- [OrcaRouter launch analysis](https://www.orcarouter.ai/blog/space-bunny-alpha-leak) — a useful contemporaneous technical reading, though not an independent benchmark.
- [TPS launch report](https://tpsreport.news/news/stealth-space-bunny-alpha-openrouter) — early coverage with telemetry, but its figures may be stale because OpenRouter’s live metrics change over time.
- [OpenRouter Stealth model archive](https://openrouter.ai/stealth) — useful for comparing Space Bunny Alpha with Union Alpha and Ox Alpha and for seeing how anonymous listings are later revealed.

[^1]: [Models search for `stealth-bunny-alpha` — OpenRouter, accessed 2026-09-24](https://openrouter.ai/models?q=stealth-bunny-alpha)
[^2]: [Space Bunny Alpha — API Pricing & Providers — OpenRouter, accessed 2026-09-24](https://openrouter.ai/stealth/space-bunny-alpha)
[^3]: [OpenRouter APIs — Space Bunny Alpha (`stealth/space-bunny-alpha`) — OpenRouter, accessed 2026-09-24](https://openrouter.ai/stealth/space-bunny-alpha/llms.txt)
[^4]: [`Stealth` model: Quasar Alpha — OpenRouter Blog, 2025-04-03](https://openrouter.ai/blog/announcements/stealth-model-quasar-alpha)
[^5]: [`Stealth` model: Optimus Alpha — OpenRouter Blog, 2025-04-10](https://openrouter.ai/blog/announcements/stealth-model-optimus-alpha)
[^6]: [Stealth Program End User License Agreement — OpenRouter, updated 2026-09-14](https://openrouter.ai/terms/stealth)
[^7]: [Stealth provider models — OpenRouter, accessed 2026-09-24](https://openrouter.ai/provider/stealth)
[^8]: [Space Bunny Alpha — AI Model Details & Benchmarks — Benchable, accessed 2026-09-24](https://benchable.ai/models/stealth/space-bunny-alpha)
[^9]: [Space Bunny Alpha: A Free 1M-Token Anonymous Model Nobody Has Claimed Yet — OrcaRouter, 2026-09-23](https://www.orcarouter.ai/blog/space-bunny-alpha-leak)
[^10]: [Provider Logging — OpenRouter Documentation, accessed 2026-09-24](https://openrouter.ai/docs/features/privacy-and-logging)
[^11]: [Anonymous Stealth Model “Space Bunny Alpha” Debuts on OpenRouter — TPS, 2026-09-23](https://tpsreport.news/news/stealth-space-bunny-alpha-openrouter)
