# Gotchas & Engineered Resolutions

## Regex Backreferences Not Supported by `regex` Crate
- **Symptom**: Background worker loop silently crashes on first fact verification task; jobs remain stuck in `running` or `queued` status forever. The thread panics with:
  `called Result::unwrap() on an Err value: regex parse error: backreferences are not supported`
- **Root Cause**: `regex::Regex` does not support backreferences like `(?is)<(script|style|iframe)\b[^>]*>[\s\S]*?</\1>`.
- **Resolution**: Use explicit alternation without backreferences:
  `(?is)<script\b[^>]*>[\s\S]*?</script>|<style\b[^>]*>[\s\S]*?</style>|<iframe\b[^>]*>[\s\S]*?</iframe>|<noscript\b[^>]*>[\s\S]*?</noscript>|<svg\b[^>]*>[\s\S]*?</svg>|<iframe\b[^>]*>`
  Additionally, enclose per-job worker execution inside `tokio::spawn` to catch any runtime panic and prevent background worker thread termination.

## Google Speech API Lack of Bulgarian Sentence Boundaries
- **Symptom**: Google AI speech recognition produces high Bulgarian lexical accuracy but outputs unpunctuated text without sentence boundaries.
- **Root Cause**: Web Speech API does not punctuate or divide sentences for Bulgarian.
- **Resolution**: Whisper runs via Vulkan GPU in ~1.2s to generate acoustic sentence boundaries, timestamps, and punctuation. The sequence alignment engine (`align_google_words_to_whisper_segments`) aligns Google's speech recognition text into Whisper's acoustic sentence segments, keeping exact timestamps and sentence punctuation. If Google speech recognition fails, Whisper's complete transcript is used as a fallback. For non-Bulgarian audio, audio routes directly to Whisper.

## SQLite Transcripts Table Check Constraint Failure
- **Symptom**: Videos take 10+ minutes stuck in processing; jobs in `ACQUIRE` repeatedly crash after full download and speech recognition with: `error returned from database: (code: 275) CHECK constraint failed: source IN ('found','generated')`.
- **Root Cause**: `crates/providers/src/transcriber.rs` returned `source: "asr".to_string()`, which violates the SQLite table constraint `source IN ('found','generated')` on table `transcripts`.
- **Resolution**: Updated `transcriber.rs` to set `source: "generated".to_string()` and added defensive fallback mapping in `engine.rs` before `db.save_transcript`.

## Worker Loop Serialization & Sequential Frame OCR
- **Symptom**: Pipeline throughput capped at 1 video at a time regardless of available CPU/GPU hardware; 20-frame OCR takes 8-10 seconds per video.
- **Root Cause**: `spawn_worker_loop` in `crates/pipeline/src/engine.rs` directly awaited each job on the loop instead of spawning tasks concurrently under `AimdConcurrencyManager` stage permits. Additionally, `crates/providers/src/ocr.rs` ran `ocr_single_frame` sequentially.
- **Resolution**: Refactored `spawn_worker_loop` to launch claimed jobs in independent `tokio::spawn` tasks with stage AIMD permits, and parallelized keyframe OCR using `futures::future::join_all`.

## Interactive "Research More" Queue Starvation & UI Inactivity
- **Symptom**: Clicking the "Проучи" (Research More) button appears to do nothing; no UI state change, toast, or spinner appears, and the video's verification never completes.
- **Root Cause**:
  1. `handle_decision` in `crates/pipeline/src/server.rs` merely enqueued `Stage::ResearchMore` into the DB `jobs` table without initiating execution.
  2. In `crates/core/src/db.rs`, `claim_next_job` sorted strictly by `j.id ASC`. Retrying failing background jobs (e.g. yt-dlp 429/captcha blocks) with low IDs monopolized the FIFO queue, completely starving newly enqueued interactive research jobs.
  3. In `ui/src/components/VirtualInbox.svelte`, `selectedCard` referenced `filteredCards` before declaration (TDZ), and the button had no loading state, disabled state, or spinner.
- **Resolution**:
  1. `handle_decision` now spawns `run_research_more_job` immediately in a background `tokio::spawn` task while updating the job status to `running` and emitting live SSE events.
  2. `claim_next_job` in `crates/core/src/db.rs` prioritizes `Stage::ResearchMore` (Priority 0) and sorts by `j.attempts ASC, j.id ASC` to prevent failing retries from blocking fresh tasks.
  3. `VirtualInbox.svelte` and `LeadCard.svelte` now display an animated spinner, disable the button during active research, show a progress track, and `App.svelte` displays live toast notifications.

## Single-Turn JSON Truncation in 10k-Word OSINT Generation
- **Symptom**: Dossiers requested to be 5,000–10,000 words generated only ~200–500 words despite system prompts asking for 10k words.
- **Root Cause**: When LLMs output structured JSON, token budgets and JSON syntax completion pressures cause them to truncate text strings to 1–2 paragraphs to maintain syntactic integrity.
- **Resolution**: Refactored `handle_research_person` to synthesize the dossier across 3 sequential chapter blocks (Chapters 1–3, 4–6, 7–9), generating 2,500–3,500 words per block. Each block is concatenated into an exhaustive 10,000-word investigative markdown document, followed by a lightweight schema metadata extraction pass.

## OpenSanctions and Live Dynamic Media Stream Resiliency
- **Symptom**: Relying on static whitelists for OSINT news and hardcoded sanction bodies misses breaking investigations, regional entities, and new PEP declarations.
- **Root Cause**: Hardcoded search templates and fixed jurisdiction filters cannot keep up with real-world corporate restructuring, new sanctions, or breaking news.
- **Resolution**: Implemented `leadfinder_providers::SanctionsMatcher` (live OpenSanctions API with multi-jurisdiction fallback) and `leadfinder_providers::DynamicNewsStream` (real-time Google News RSS, BTA, and BNR parser with entity deduplication and background worker polling). Corporate entities (`ООД`, `ЕООД`, `АД`, `Холдинг`) are extracted on the fly via `extract_entity_pivots` for 2nd-degree recursive sub-searches.

## Intra-Video Dialectical Denial Fallacy
- **Symptom**: Claims from dialogue or debate videos were marked `CONTRADICTED` with label `false_claim` simply because another speaker in the video said "I never said that" or "That is a lie".
- **Root Cause**: `prompts/verify_v1.txt` contained `"лицето X никога не е казало/гласувало/договорило Y, или го е отрекло"`, which explicitly prompted the LLM to interpret a speaker's denial as empirical proof of falsehood.
- **Resolution**: Removed the denial clause and introduced the Dialectical Anti-Fallacy Rule in `prompts/verify_v1.txt`. In `enforce_verification_rules`, if contradiction is based solely on verbal disputes or denials between speakers without an external primary document, official register, or fact-check, the verdict is forced to `UNVERIFIABLE`.

## Hidden Puncture Web URLs & Search Retrieval Blindspots
- **Symptom**: Fact-checking reasoning never cited exact external links, and verification failed to find evidence for breaking or political claims.
- **Root Cause**: `build_verify_user_prompt` printed `p.source_name.as_deref().unwrap_or(&p.source_url)`, which hid `p.source_url` because `source_name` was always populated. Furthermore, `is_untrustworthy_or_junk_url` discarded `news.google.com` article URLs, and Bing RSS stripped advanced `site:` query chunks.
- **Resolution**: Explicitly print `Източник URL: {}` for every retrieved passage in `build_verify_user_prompt`. Added DuckDuckGo Lite table parsing and integrated `DynamicNewsStream::search_news_hits` directly into claim verification retrieval.

## Wayback Machine CDX Wildcard Timeouts & Exact Domain Optimization
- **Symptom**: Querying `web.archive.org/cdx/search/cdx` with wildcard domains like `*domain*` timed out after 30+ seconds.
- **Root Cause**: Internet Archive CDX server struggles with wildcard prefix/suffix queries across millions of URLs.
- **Resolution**: Use exact host names without wildcards (`url=domain.com`) with `matchType=domain` and `limit=5`, or use the Availability JSON API (`https://archive.org/wayback/available?url={url}`).

## Wikimedia REST / API 403 Forbidden Without Custom User-Agent
- **Symptom**: Querying Wikipedia API (`bg.wikipedia.org/w/api.php`) via standard HTTP client returned HTTP 403 Forbidden.
- **Root Cause**: Wikimedia Foundation policy requires all API consumers to supply a descriptive `User-Agent` header; generic clients or Python urllib default User-Agents are blocked.
- **Resolution**: Configured a custom descriptive User-Agent (`LeadFinder-OSINT/1.0 (contact@leadfinder.bg; security-research)`) on all Wikimedia and Wayback requests.

## OSINT Web Crawl Politeness Starvation During Deep Dossier Generation
- **Symptom**: Generating deep OSINT dossiers took >180 seconds, triggering HTTP timeout on client requests.
- **Root Cause**: Running 10 sequential web queries with a politeness limiter of 12 requests/min forced each query to wait 5 seconds. Downloading 15 HTML pages sequentially compounded the delay to over 2 minutes.
- **Resolution**: Limited external live search to top 2 focused queries with max 2 hits per query, while relying on direct Wikipedia Bulgarian API extracts (50,000+ characters fetched in <1s), Google News RSS feeds, and Wayback archives. Wrapped each LLM block request in a 12-second timeout to fall back to instant full encyclopedic section aggregation, completing 10,800-word dossiers in ~45 seconds.

## Cross-Hashtag Delimitation & Garbage Entity Conflation
- **Symptom**: People directory populated with composite pairs and garbage names: `Пеевски Борисов`, `Делянпеевски Политика`, `Пеевски Който`, `Руменрадев Пеевски`, `Радославревански Делянпеевски`.
- **Root Cause**:
  1. Social captions concatenating hashtags (e.g. `#РуменРадев #Пеевски #Борисов` or `#делянпеевски #политика`) were stripped of `#` and passed directly into `split_whitespace()`, treating consecutive hashtags as adjacent words in a single capitalized name.
  2. Commas in clauses (e.g. "Делян Пеевски, който...") were stripped, conflating the relative pronoun `който` into the person's canonical name (`Пеевски Който`).
- **Resolution**:
  1. Replaced `#`, commas, and colons with sentence-terminating boundaries (`. `) in `sanitize_text_for_name_scanning` before running pair scanner.
  2. Implemented `leadfinder_providers::EntityCleaner` with Bulgarian stopwords, relative pronouns, honorifics (`господин`, `г-н`), and category topic words (`политика`, `новини`).
  3. Added Google Contacts style deduplication engine (`/api/people/dedup/proposals`, `/api/people/dedup/apply`, `/api/people/dedup/auto`) supporting automatic merges, compound splitting, and tag extraction.
  4. Built top-bar notification center modal with interactive questions, system notifications, and video media playback controls.

## UTF-8 Byte Slicing Panic in Cyrillic Text & Quotes
- **Symptom**: Background worker thread panics during entity pivoting or news processing with:
  `thread 'tokio-rt-worker' panicked at crates\providers\src\people.rs: start byte index 225 is not a char boundary; it is inside 'в' (bytes 224..226 of string)`
- **Root Cause**:
  1. `abs_pos.saturating_sub(40)` subtracted raw byte offsets, splitting 2-byte Cyrillic characters in half.
  2. Indexing past multi-byte Bulgarian quotes (`q_start + 1` where `„` is 3 bytes in UTF-8: `E2 80 9E`) landed on an invalid interior byte boundary.
- **Resolution**: Converted byte offsets to safe `char`-based windows (`prefix.chars().collect::<Vec<char>>()`), taking character counts (`take_count`) rather than raw bytes, and consumed opening quote characters using iterator step (`q_chars.next()`). Added unit test `test_extract_entity_pivots_cyrillic_boundary_safe`.

- PowerShell reports exit code 1 for 'cargo ... 2>&1' when cargo prints Finished to stderr; judge by Finished/test result lines.
- Release build fails with 'failed to remove file target\release\leadfinder.exe' while leadfinder.exe or leadfinder-desktop.exe runs; ask user, stop them, rebuild.
- Svelte allows one style block per file; insert CSS before the existing closing style tag in DetailDrawer.svelte.
- Pair -webkit-line-clamp with standard line-clamp to avoid svelte-check warnings.
- Claim redaction: POST /api/claims/:id/redact -> update_claim_text (clears verdict) -> feedback rating 'edited' -> verify_claim_with_retrieval injects latest_claim_notes as REVIEWER_NOTES (guidance, not evidence).
