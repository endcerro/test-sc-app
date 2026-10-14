# OCR of hauling contracts

Goal: read a mobiGlas **Contracts > Offers** screenshot offline and get a structured
`HaulingContract` (reward, contractor, container size, cargo lines with pickups and drop-off)
for the cargo route planner.

```text
screenshot ──► sc-ocr::layout ──► crops ──► sc-ocr::preprocess ──► OcrEngine ──► text
                (regions, bullets)            (Pipeline)            (paddle/ocrs/winocr/tesseract)
text ──► sc-contracts::parse (+ repair, merge, snap) ──► HaulingContract ──► score vs truth
```

## The screen

All samples are 1920x1080. The UI is fixed, but the panel moves by a few pixels between
shots (up to 6 px in the samples).

| Region | Content | Colours |
| --- | --- | --- |
| `title` | contract title, 1-2 lines, condensed font | white + blue `[100 Rep]` / `[BP]*` |
| `reward` | values column: reward, availability (`N/A`, `2h 1m`), contractor | white |
| `details` | free text: full place names (`Ruin Station above Pyro VI`), container size, `Reputation Awarded` | white + blue links |
| `objectives` | `Deliver 0/N SCU of X to Y.` / `Collect X from Z.` bullets | white, pale-yellow hollow diamonds |

What the parser takes from each:

- **objectives**: cargo lines. Each `Deliver` takes the `Collect`s that follow it; several
  `Collect`s for one `Deliver` become several pickups (sample 07). A `Deliver` whose `Collect`
  scrolled off gets the contract's most common pickup, marked uncertain (`?`).
- **details**: `MAXBOX` is the first `N SCU` (`16 SCU cargo containers`, `Max size will be 8
  SCU`, `4 SCU or smaller`, `largest container used is 2 SCU`). `REP` is `Reputation Awarded`.
  Short `Collect` names are completed from `<name> above|on|in|within <parent> [numeral]`
  (`Ruin Station` -> `Ruin Station above Pyro VI`). `at` is ignored on purpose: `Wide Forest
  Station at ArcCorp's L1 Lagrange point` is a description, not a name.
- **reward / title**: as displayed (title lines joined).
- Scrolled lists (09-1 + 09-2) are merged by aligning the longest run of identical bullets
  that ends where the previous shot ends (`sc_contracts::merge`).

With perfect text the parser reproduces every truth file exactly
(`crates/sc-contracts/tests/transcripts.rs`).

## Helping the engines

Layout (`sc-ocr::layout`):

- Regions are defined in a 1920x1080 reference frame and scaled by height (UI assumed 16:9
  and centred, so ultrawide works by offset).
- **Anchor**: the green `OFFERS` tab is found by colour, and its offset shifts every region.
- **Bullets**: the pale-yellow diamonds (`#fdf4b9`; the UI white is bluish, so "blue well
  below red/green" separates them) are found in the left gutter of the objectives. The big,
  left ones are `Deliver` and the small, indented ones are `Collect`. Faded bullets at scroll
  edges are too dark and are skipped, which drops the half-visible ghost lines in 09-2. One
  crop per bullet gives the `lines` crop mode.

Preprocessing axes (`sc-ocr::preprocess::Pipeline`, every combination is benchmarked):

| Axis | Values |
| --- | --- |
| colour | `rgb`, `gray`, `max` (max channel, keeps blue text bright), `white` (near-white pixels only), `whiteblue` (white + link blue); masks always drop the yellow bullets |
| background | none, `tophat` (subtract a 9 px opening: removes the sun glow and rays) |
| scale | x1, x2, x3 (Lanczos) |
| binarise | none, `otsu`, `sauvola` |
| despeckle | small connected components on masked/binary images, 3x3 median otherwise |
| polarity | native (light on dark), `inv` (dark on white) |

Crop modes: `full` (whole screenshot, lines assigned to regions by box centre; x1 only),
`fixed` (reference rectangles), `anchored` (shifted by the OFFERS tab), `lines` (objectives
only, one crop per bullet; Paddle skips detection there and recognises projection-split rows).

Text repairs (`sc-contracts::repair`), applied only where the content type is known:

- numbers: `O`->0, `B`->8, `l/I`->1, `S`->5 ... in SCU, reward and rep
- `[!]` restricted marker read as `[1]`, `(!)`, `[I]`
- keyword fuzz (`Dellver`), bullet junk before keywords
- names: trailing junk dropped, stylised `&` restored (`Samson 6 Son's`), roman numerals after
  a capitalised word (`Pyro Ill` -> `Pyro III`, `Pyro ?I` -> `Pyro II`)
- optional snapping: commodities to `crates/sc-contracts/data/commodities.json` (scunpacked
  `resources/commodities.json`, game 4.10.2), places to the `Universe` (names + aliases)

## Engines

All run offline once the models are downloaded (`ocr-bench fetch-models`; URLs and sha256 are
pinned in `crates/sc-ocr/src/models.rs`).

| Engine | Feature | What | Size |
| --- | --- | --- | --- |
| `paddle/v5srv+en` | `paddle` | PP-OCRv5 server detection + PP-OCRv5 English recognition, ONNX Runtime (`ort`) | 88 + 7.5 MB |
| `paddle/v3+en` | `paddle` | PP-OCRv3 mobile detection + v5 English recognition | 2.4 + 7.5 MB |
| `paddle/v5srv+latin` | `paddle` | v5 server detection + v5 Latin recognition | 88 + 7.5 MB |
| `ocrs` | `ocrs` | pure Rust (rten), no native runtime | 2.5 + 9.7 MB |
| `winocr/en-US` | `winocr` | `Windows.Media.Ocr`, ships with Windows | 0 |
| `tesseract/psmN[/dict]` | `tesseract` | Tesseract 5 CLI, LSTM, PSM 4/6/11, optional `--user-words` (commodity + place words) | external install |

ONNX Runtime binaries are downloaded by `ort` at build time and copied next to the binary.

## Benchmark

```sh
cargo run -p ocr-bench --release --features all-engines -- fetch-models
cargo run -p ocr-bench --release --features all-engines -- run --preset quick     # ~5 min
cargo run -p ocr-bench --release --features all-engines -- run --universe         # reduced (default), ~25 min
cargo run -p ocr-bench --release --features all-engines -- run --resize 2560x1440 --pipelines <id> --engines <name>
cargo run -p ocr-bench --release -- dump --sample 05 --pipeline whiteblue/-/x2/otsu/-/inv
```

### Full grid (run it overnight)

```sh
cargo run -p ocr-bench --release --features all-engines -- run --universe --preset full --name full
```

Every pipeline (300) x every crop mode x all 10 engine configs x every sample: 100 000 units.
On a Ryzen 7 9800X3D the PP-OCRv5 server detector alone runs about 0.8 s per unit (all cores),
so expect **15-20 h**. It is resumable: stop it any time, and the next run only computes
what `target/ocr-bench/cache.jsonl` does not have yet (units from `reduced` runs are reused).
Results land in `target/ocr-bench/full/summary.md`.

Presets (`--preset`):

| Preset | Pipelines | Crop modes | Engine configs | Units (10 shots) |
| --- | --- | --- | --- | --- |
| `quick` | 20 (rgb/gray/whiteblue, no top-hat, no Sauvola, x1-x2) | anchored, lines | 1 per family | ~1 600 |
| `reduced` | 68 cross-product (every colour x top-hat x {none, Otsu} x {x1, x2} x polarity) + 11 one-axis variants of 4 baselines (x3, Sauvola, despeckle) | anchored, lines; fixed and full on 3 baselines | paddle v5srv+en, v3+en; ocrs; winocr; tesseract psm6, psm11, psm6/dict | ~11 500 |
| `full` | 300 (all) | all four | all 10 (adds paddle v5srv+latin, tesseract psm4, psm4/dict) | 100 000 |

- Truth: `samples/truth/<stem>.txt` (`#HAULING v1`, end-to-end fields) and
  `samples/truth/regions/<stem>/<region>.txt` (exact text, for CER). Region transcripts leave
  out the faded half-scrolled lines and the currency glyph.
- Metrics: CER/WER per region (whitespace and quotes normalised, dash rules shortened);
  end to end, accuracy = share of scalar fields + cargo columns (scu, commodity with `[!]`,
  pickups, drop-off) right after order-preserving alignment, and F1 over exact cargo lines.
  Rankings show the mean and the worst sample.
- Every OCR result is cached in `target/ocr-bench/cache.jsonl` (key: image hash, crop mode,
  pipeline, engine config, `CACHE_VERSION`), so changing the parser only re-scores.
- Latency is measured afterwards, one screenshot at a time (warm), since the parallel grid
  timings are distorted by engines sharing the thread pool.
- Outputs: `target/ocr-bench/<run>/summary.md` and `results.csv`.

## Results

Reduced preset, 2026-10-14: 10 screenshots / 10 cases (the two 09 shots also scored merged),
7 engine configs, 11 480 units, 46 min on a Ryzen 7 9800X3D. The full report is
`target/ocr-bench/reduced/summary.md`. Region CER is mean (worst) over the 10 shots.

### Recommendation

**PaddleOCR, PP-OCRv3 mobile detection + PP-OCRv5 English recognition (`paddle/v3+en`),
anchored crops, pipeline `max/-/x2/-/despeckle/native`** (max-channel grey, x2 Lanczos, 3x3
median, light-on-dark kept), commodity + place snapping on:

- **10/10 contracts parsed exactly** (every field and cargo line), with one setting for all
  regions. Its neighbours are almost as good (no despeckle 99.2% field accuracy, x3 99.2%,
  x1 98.5%), so it is not a lucky point.
- ~10 MB of models, no GPU, no external install. The ONNX Runtime DLL ships with the app.
- About 2.5 s per screenshot when Paddle sessions are single-threaded (the bench setting); details is the
  slowest region (1.3 s). The app can give the sessions more threads (not measured yet).
- Scale should follow resolution: aim for ~2160 px of equivalent screen height
  (`scale = round(2160 / height)`, clamped to 1..3). On resampled samples: 3840x2160 at x1 or x2 got
  10/10, 2560x1440 at x1 got 9/10 (99.6%), and 1280x720 tops out at ~94% even at x3 (the text is
  just too small).

### End to end (snapping on)

| Engine | Best CER per region | Field-optimised per region |
| --- | --- | --- |
| paddle/v3+en | 98.5%, 9/10 exact | **100%, 10/10** |
| paddle/v5srv+en | 95.4%, 4/10 | 97.6%, 6/10 |
| ocrs | 92.1%, 4/10 | 98.9%, 9/10 |
| tesseract/psm6/dict | 89.4%, 2/10 | 96.1%, 8/10 |
| tesseract/psm6 | 90.4%, 1/10 | 95.1%, 6/10 |
| tesseract/psm11 | 83.1%, 2/10 | 89.8%, 3/10 |
| winocr/en-US | 73.2%, 0/10 | 78.1%, 1/10 |
| mixed engines | 96.0%, 5/10 | 100%, 10/10 |

With one config for all four regions, the 15 best configs are all `paddle/v3+en` (96.5% to
100%). The best is the recommended one.

"Field-optimised" swaps each region's config to maximise field accuracy on these samples. It
is fitted, so treat it as an upper bound. Picking by CER alone is not enough: on sample 02 the
lowest-CER details read `Pyro II` for `Pyro III`, while a config with slightly higher CER reads
`Pyro Ill`, which the repair fixes.

### Per region (best config of each engine, CER)

| Engine | Title | Reward | Details | Objectives |
| --- | --- | --- | --- | --- |
| paddle/v3+en | 0.5% (2.3) | 0.9% (3.4) | 0.3% (0.8) | 1.1% (2.1) |
| paddle/v5srv+en | 0.6% (3.4) | 0.2% (2.3) | 0.3% (0.9) | 1.7% (2.7) |
| ocrs | 5.8% (9.2) | 5.6% (8.7) | 1.2% (1.9) | **0.7%** (2.3), `lines` |
| tesseract/psm6 | **0.2%** (1.3) | 4.1% (9.1) | 0.7% (1.3) | 1.5% (4.6), `lines` |
| winocr/en-US | 2.6% (5.1) | 14.1% (27.6) | 1.6% (3.3) | 5.2% (9.4), `lines` |

### What the preprocessing axes do

- **Engine choice matters most.** The PP-OCR models read this UI almost raw. Tesseract and
  Windows OCR need help.
- **Colour**: `max` (max channel) is the best all-rounder: blue links stay as bright as white
  text. The white-only mask (the "text is always white" idea) is fine on objectives (best mean
  CER there, 8.9%) but loses the blue link text in details (21% CER at best) and the title, so it
  only fits the objectives region.
- **Binarisation** (Otsu, Sauvola) hurts the neural engines on average and only helps
  Tesseract on titles. Their models expect grey levels.
- **Top-hat** (glow removal) helps Tesseract and Windows OCR (their best configs use it).
  It does little for Paddle/ocrs.
- **Scale**: x2 is the sweet spot at 1080p. x3 rarely adds anything and costs ~2x.
- **Polarity** barely matters for the neural engines. Tesseract prefers inverted (dark on light)
  only for the title.
- **Crop mode**: `full` (whole screenshot) is far worse on average (30-45% CER: neighbouring
  panels leak in, and the region assignment of lines fails), so cropping per region is the key
  trick. `fixed` ~= `anchored` on these samples (shift <= 6 px, inside the 8 px margin), so the
  anchor is insurance for other setups. `lines` (one crop per bullet) gives the best
  objectives for ocrs, Tesseract and Windows OCR, and is the fastest Paddle path (no detection).
- **Repairs** carry a lot: `[1]`->`[!]`, `Pyro Ill`->`Pyro III`, `Samson 6 Son's`->`&`,
  `0/8O`->80, `2 ScU`. Snapping commodities and places adds 0.5-5 points depending on the engine.
  Place snapping only fixes the location name, never the parent: contracts say `Pyro VI` where
  the starmap says `Terminus`.

### Caveats

- 10 samples, 9 contracts, one 1080p setup (same HUD scale, same language). Other resolutions
  were simulated by resampling. Real captures at 1440p/4K/720p and other HUD settings
  should be added to `samples/`.
- In the reduced preset x3, Sauvola and despeckle are only tried on 4 strong baselines, so their
  axis averages look better than they would in the full grid.
- Follow-ups: add planet designations (`Pyro III` = Bloom) as aliases in sc-data so parents can
  be checked against the universe; capture the screen directly (hotkey) instead of dropping
  screenshot files; read the `|` title separators reliably.

## In the app

The **Contracts** pane of `sc-companion` (and its `contract` CLI command) runs the
recommended setup: `paddle/v3+en`, anchored crops, `sc_ocr::recommended_pipeline(height)`
(`max/-/xN/-/despeckle/native`, N from the screen height), commodity + place snapping against
the loaded universe.

```sh
cargo run -p sc-companion --release --features ocr                  # drop screenshots on the Contracts pane
cargo run -p sc-companion --release --features ocr -- contract samples/03-redwind-ruin-baijini.webp
```

- The `ocr` feature is off by default (CI stays ONNX-free); without it the pane lists saved
  contracts and says the build has no OCR.
- The three model files (~10 MB) are downloaded on the first read into the same model dir
  the benchmark uses (`sc_ocr::models::default_dir`), checked against their pinned sha256.
- The four regions are read concurrently (4 sessions per model): about 1.5-2 s per 1080p
  screenshot on a Ryzen 7 9800X3D, plus ~0.5 s to load the engine on the first one.
- Every dropped file is read on a background thread, one at a time, then parsed and saved in
  `<data dir>/sc-companion/contracts` (override with `--contracts-dir`): `<id>.txt` is the
  contract in `#HAULING v1` (+ `# source:` comments), `<id>.ocr.json` the raw region text.
  A screenshot whose title, reward and contractor match a saved contract is merged into it
  (`contract_from_shots`), so dropping 09-1 then 09-2 gives the merged truth.
- On the samples the app output matches the truth files except that the thin `|` title
  separators are sometimes not read (01, 02, 06); scoring ignores punctuation in titles.

## Adding samples

Drop `NN-name.webp|png` in `samples/`, write `truth/NN-name.txt` (`#HAULING v1`) and, for
CER, `truth/regions/NN-name/{title,reward,details,objectives}.txt` (use
`ocr-bench dump --sample NN` to get the crops). Scrolled shots are `NN-name-1`, `NN-name-2`
with the merged truth in `truth/NN-name.txt`. Then re-run the benchmark: only the new
images are OCR'd.
