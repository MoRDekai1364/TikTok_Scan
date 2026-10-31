<script lang="ts">
  import { onMount } from 'svelte';
  import { fetchCreators, runCustomTikTokScan } from '../lib/api';
  import type { Language } from '../lib/i18n';
  import type { CustomScanConfig, CustomScanResult } from '../lib/types';

  let {
    isOpen,
    currentLang,
    onClose,
    onJumpToVideo,
    onScanComplete,
  }: {
    isOpen: boolean;
    currentLang: Language;
    onClose: () => void;
    onJumpToVideo?: (id: string) => void;
    onScanComplete?: () => void;
  } = $props();

  let targetMode = $state<'watchlist' | 'creators' | 'keywords' | 'general'>('watchlist');
  let customTargetsText = $state('');
  let keywordQuery = $state('');
  let generalQuery = $state('');
  let allowNoKeywords = $state(false);
  let maxPerTarget = $state(5);
  let minViewsChoice = $state(0);
  let minLikesChoice = $state(0);
  let recencyDays = $state(7);
  let maxTargetsChoice = $state(0);
  let autoProcess = $state(true);

  let isScanning = $state(false);
  let scanResult = $state<CustomScanResult | null>(null);
  let errorMessage = $state<string | null>(null);
  let elapsedSec = $state(0);
  let timerRef: ReturnType<typeof setInterval> | null = null;
  let watchlistCount = $state(23);

  onMount(() => {
    fetchCreators().then((list) => {
      const active = list.filter((c) => c.watchlist).length;
      if (active > 0) watchlistCount = active;
    }).catch(() => {});
  });

  function startTimer() {
    elapsedSec = 0;
    if (timerRef) clearInterval(timerRef);
    timerRef = setInterval(() => {
      elapsedSec += 1;
    }, 1000);
  }

  function stopTimer() {
    if (timerRef) {
      clearInterval(timerRef);
      timerRef = null;
    }
  }

  function applyPreset(name: string) {
    if (name === 'full') {
      targetMode = 'watchlist';
      allowNoKeywords = true;
      keywordQuery = '';
      maxPerTarget = 5;
      minViewsChoice = 0;
      recencyDays = 7;
      maxTargetsChoice = 0;
      autoProcess = true;
    } else if (name === 'fast') {
      targetMode = 'watchlist';
      allowNoKeywords = true;
      keywordQuery = '';
      maxPerTarget = 3;
      minViewsChoice = 1000;
      recencyDays = 3;
      maxTargetsChoice = 5;
      autoProcess = true;
    } else if (name === 'open') {
      targetMode = 'general';
      allowNoKeywords = true;
      keywordQuery = '';
      generalQuery = '';
      maxPerTarget = 5;
      minViewsChoice = 0;
      recencyDays = 7;
      maxTargetsChoice = 0;
      autoProcess = true;
    } else if (name === 'viral') {
      targetMode = 'watchlist';
      allowNoKeywords = true;
      keywordQuery = '';
      maxPerTarget = 10;
      minViewsChoice = 10000;
      minLikesChoice = 200;
      recencyDays = 7;
      maxTargetsChoice = 0;
      autoProcess = true;
    }
  }

  async function handleStartScan() {
    if (isScanning) return;
    errorMessage = null;
    scanResult = null;
    isScanning = true;
    startTimer();

    try {
      let targets: string[] = [];
      if (targetMode === 'creators') {
        targets = customTargetsText
          .split(/[\n,; ]+/)
          .map((s) => s.trim())
          .filter((s) => s.length > 0);
        if (targets.length === 0) {
          errorMessage = currentLang === 'bg'
            ? 'Моля въведете поне един профил на автор (напр. @darik_bg)'
            : 'Please enter at least one creator handle (e.g. @darik_bg)';
          isScanning = false;
          stopTimer();
          return;
        }
      } else if (targetMode === 'general') {
        if (!allowNoKeywords && generalQuery.trim().length > 0) {
          targets = generalQuery
            .split(/[\n,;]+/)
            .map((s) => s.trim())
            .filter((s) => s.length > 0);
        } else {
          targets = [];
          allowNoKeywords = true;
        }
      } else if (targetMode === 'keywords') {
        if (!allowNoKeywords && keywordQuery.trim().length > 0) {
          targets = keywordQuery
            .split(/[\n,;]+/)
            .map((s) => s.trim())
            .filter((s) => s.length > 0);
        } else {
          targets = [];
          allowNoKeywords = true;
        }
      }

      const effectiveAllowNoKeywords = allowNoKeywords || targets.length === 0;

      const config: CustomScanConfig = {
        target_mode: targetMode,
        targets,
        allow_no_keywords: effectiveAllowNoKeywords,
        max_per_target: Number(maxPerTarget),
        min_views: minViewsChoice > 0 ? minViewsChoice : null,
        min_likes: minLikesChoice > 0 ? minLikesChoice : null,
        max_age_days: recencyDays > 0 ? recencyDays : null,
        max_targets: maxTargetsChoice > 0 ? maxTargetsChoice : null,
        auto_process: autoProcess,
      };

      const res = await runCustomTikTokScan(config);
      scanResult = res;
      if (onScanComplete) {
        onScanComplete();
      }
    } catch (e: any) {
      errorMessage = e?.message || (currentLang === 'bg' ? 'Грешка при сканиране' : 'Scan failed');
    } finally {
      isScanning = false;
      stopTimer();
    }
  }

  function handleJump(id: string) {
    if (onJumpToVideo) {
      onJumpToVideo(id);
    }
    onClose();
  }

  function formatViews(n?: number | null): string {
    if (n === null || n === undefined) return '0';
    if (n >= 1000000) return (n / 1000000).toFixed(1) + 'M';
    if (n >= 1000) return (n / 1000).toFixed(1) + 'k';
    return n.toLocaleString();
  }
</script>

{#if isOpen}
  <div class="modal-backdrop" onclick={onClose} role="presentation">
    <div
      class="modal-dialog scan-modal"
      onclick={(e) => e.stopPropagation()}
      onkeydown={(e) => e.stopPropagation()}
      role="dialog"
      tabindex="-1"
    >
      <div class="modal-head">
        <div class="modal-title-col">
          <div class="title-row">
            <h3>🎯 {currentLang === 'bg' ? 'Търсене на видеа в TikTok' : 'TikTok Video Search'}</h3>
            <span class="scan-mode-badge">
              ⚡ {currentLang === 'bg' ? 'Директно търсене' : 'Direct Search'}
            </span>
            <span class="scan-wl-badge">
              📺 {watchlistCount} {currentLang === 'bg' ? 'наблюдавани канала' : 'watchlist channels'}
            </span>
          </div>
          <span class="scan-sub">
            {currentLang === 'bg'
              ? 'Откриване и филтриране на актуални видеа по канали, ключови думи и минимален брой гледания'
              : 'Autonomous discovery and filtering of fresh video content with customized parameters'}
          </span>
        </div>

        <button class="close-btn" onclick={onClose} aria-label="Close">✕</button>
      </div>

      <div class="modal-body">
        <div class="presets-bar">
          <span class="presets-label">⚡ {currentLang === 'bg' ? 'Готови настройки:' : 'Quick Presets:'}</span>
          <button class="preset-btn" onclick={() => applyPreset('fast')}>
            ⚡ {currentLang === 'bg' ? 'Бърза проверка (5 канала, 3 видеа)' : 'Quick Test (5 channels, 3 vids)'}
          </button>
          <button class="preset-btn" onclick={() => applyPreset('open')}>
            🌐 {currentLang === 'bg' ? 'Общо търсене (без филтър)' : 'Open Scan (no filter)'}
          </button>
          <button class="preset-btn" onclick={() => applyPreset('full')}>
            📺 {currentLang === 'bg' ? 'Всички наблюдавани канали' : 'Full Watchlist'}
          </button>
          <button class="preset-btn" onclick={() => applyPreset('viral')}>
            🔥 {currentLang === 'bg' ? 'Популярни видеа (>10 хил. гледания)' : 'Popular Videos (>10k views)'}
          </button>
        </div>

        <div class="scan-sections">
          <div class="scan-section">
            <h4 class="section-title">
              1. {currentLang === 'bg' ? 'Цел на сканирането' : 'Scan Target'}
            </h4>
            <div class="target-mode-selector">
              <label class="mode-card" class:active={targetMode === 'general'}>
                <input
                  type="radio"
                  name="target_mode"
                  value="general"
                  checked={targetMode === 'general'}
                  onchange={() => (targetMode = 'general')}
                />
                <div class="mode-info">
                  <span class="mode-name">🌐 {currentLang === 'bg' ? 'Общо търсене (General Search)' : 'General Search'}</span>
                  <span class="mode-desc">
                    {currentLang === 'bg'
                      ? 'Търсене в TikTok по тема, личност или отворено обхождане на всички източници и автори'
                      : 'Search TikTok by topic, entity, or open scan across all sources and creators'}
                  </span>
                </div>
              </label>

              <label class="mode-card" class:active={targetMode === 'watchlist'}>
                <input
                  type="radio"
                  name="target_mode"
                  value="watchlist"
                  checked={targetMode === 'watchlist'}
                  onchange={() => (targetMode = 'watchlist')}
                />
                <div class="mode-info">
                  <span class="mode-name">📺 {currentLang === 'bg' ? 'Наблюдавани канали (Watchlist)' : 'Watchlist Creators'}</span>
                  <span class="mode-desc">
                    {currentLang === 'bg'
                      ? `Сканира всички ${watchlistCount} канала от списъка (Blitz, Darik, SafeNews, Victor Yochev и др.)`
                      : `Scans all ${watchlistCount} monitored channels in database`}
                  </span>
                </div>
              </label>

              <label class="mode-card" class:active={targetMode === 'creators'}>
                <input
                  type="radio"
                  name="target_mode"
                  value="creators"
                  checked={targetMode === 'creators'}
                  onchange={() => (targetMode = 'creators')}
                />
                <div class="mode-info">
                  <span class="mode-name">👤 {currentLang === 'bg' ? 'Конкретни автори' : 'Specific Handles'}</span>
                  <span class="mode-desc">
                    {currentLang === 'bg'
                      ? 'Въведете конкретни потребителски имена на профили за прецизно извличане'
                      : 'Enter specific creator handles for targeted scanning'}
                  </span>
                </div>
              </label>

              <label class="mode-card" class:active={targetMode === 'keywords'}>
                <input
                  type="radio"
                  name="target_mode"
                  value="keywords"
                  checked={targetMode === 'keywords'}
                  onchange={() => (targetMode = 'keywords')}
                />
                <div class="mode-info">
                  <span class="mode-name">🔍 {currentLang === 'bg' ? 'Ключови думи & Хаштагове' : 'Keywords & Hashtags'}</span>
                  <span class="mode-desc">
                    {currentLang === 'bg'
                      ? 'Търсене по избрани теми или сканиране без филтър за пълно покритие'
                      : 'Scan by chosen topics or unconstrained without filters'}
                  </span>
                </div>
              </label>
            </div>

            {#if targetMode === 'general'}
              <div class="input-subgroup">
                <div class="field-label-row">
                  <label for="general-query" class="field-label">
                    {currentLang === 'bg' ? 'Тема / Ключова фраза за общо търсене (по избор):' : 'Topic / Search phrase for general search (optional):'}
                  </label>
                  <label class="toggle-pill-label">
                    <input type="checkbox" bind:checked={allowNoKeywords} />
                    <span>⚡ {currentLang === 'bg' ? 'Без филтър (общо обхождане)' : 'No filter (broad scan)'}</span>
                  </label>
                </div>
                <input
                  id="general-query"
                  type="text"
                  class="custom-input"
                  placeholder={allowNoKeywords
                    ? (currentLang === 'bg' ? 'Широко обхождане на всички източници и автори без ограничение...' : 'Broad scan across all sources and creators without filter...')
                    : (currentLang === 'bg' ? 'избори, протест, Румен Радев, бюджет, инфлация (или оставете празно)' : 'elections, protest, budget, inflation (or leave blank)')}
                  disabled={allowNoKeywords}
                  bind:value={generalQuery}
                />
                <span class="field-hint">
                  {allowNoKeywords || generalQuery.trim().length === 0
                    ? (currentLang === 'bg' ? '✓ Общо търсене: скенерът ще обходи всички достъпни автори и източници без тематични ограничения.' : '✓ General search: scanner will traverse all available creators and sources without topical restrictions.')
                    : (currentLang === 'bg' ? '✓ Търсене в заглавия, хаштагове и описания сред цялата мрежа от източници и автори.' : '✓ Searching titles, hashtags and captions across the entire network of sources and creators.')}
                </span>
              </div>
            {/if}

            {#if targetMode === 'creators'}
              <div class="input-subgroup">
                <label for="custom-handles" class="field-label">
                  {currentLang === 'bg' ? 'Потребителски имена (разделени със запетая или нов ред):' : 'Creator handles (comma or newline separated):'}
                </label>
                <textarea
                  id="custom-handles"
                  class="custom-textarea"
                  placeholder="@victor.yochev, @darik_bg, @blitzbg, @safenews_official"
                  bind:value={customTargetsText}
                  rows={2}
                ></textarea>
              </div>
            {/if}

            {#if targetMode === 'keywords'}
              <div class="input-subgroup">
                <div class="field-label-row">
                  <label for="custom-keywords" class="field-label">
                    {currentLang === 'bg' ? 'Ключови думи / хаштагове (по избор):' : 'Keywords / hashtags (optional):'}
                  </label>
                  <label class="toggle-pill-label">
                    <input type="checkbox" bind:checked={allowNoKeywords} />
                    <span>⚡ {currentLang === 'bg' ? 'Без филтър (всички видеа)' : 'No filter (all videos)'}</span>
                  </label>
                </div>
                <input
                  id="custom-keywords"
                  type="text"
                  class="custom-input"
                  placeholder={allowNoKeywords
                    ? (currentLang === 'bg' ? 'Без филтър по хаштаг / дума (всички видеа ще бъдат сканирани)' : 'No hashtag / keyword filter (all videos will be scanned)')
                    : (currentLang === 'bg' ? 'избори, парламент, правителство, #новини (или оставете празно)' : 'elections, parliament, #news (or leave blank)')}
                  disabled={allowNoKeywords}
                  bind:value={keywordQuery}
                />
                <span class="field-hint">
                  {allowNoKeywords || keywordQuery.trim().length === 0
                    ? (currentLang === 'bg' ? '✓ Без филтър: скенерът ще обходи каналите без ограничение по хаштаг или ключова дума.' : '✓ No-filter mode: scanner will ingest videos without hashtag or keyword restrictions.')
                    : (currentLang === 'bg' ? 'Филтриране по съвпадение в заглавието или хаштаговете.' : 'Filtering by match in caption or hashtags.')}
                </span>
              </div>
            {/if}
          </div>

          <div class="scan-section">
            <h4 class="section-title">
              2. {currentLang === 'bg' ? 'Филтри и прецизност' : 'Filters & Precision'}
            </h4>
            <div class="filters-grid">
              <div class="filter-box">
                <span class="filter-label">🎬 {currentLang === 'bg' ? 'Видеа на канал:' : 'Videos per creator:'}</span>
                <div class="pill-group">
                  {#each [3, 5, 10, 15] as v}
                    <button
                      type="button"
                      class="pill-choice"
                      class:active={maxPerTarget === v}
                      onclick={() => (maxPerTarget = v)}
                    >
                      {v}
                    </button>
                  {/each}
                </div>
              </div>

              <div class="filter-box">
                <span class="filter-label">👁️ {currentLang === 'bg' ? 'Мин. гледания:' : 'Min views:'}</span>
                <div class="pill-group">
                  <button
                    type="button"
                    class="pill-choice"
                    class:active={minViewsChoice === 0}
                    onclick={() => (minViewsChoice = 0)}
                  >
                    {currentLang === 'bg' ? 'Всички' : 'All'}
                  </button>
                  <button
                    type="button"
                    class="pill-choice"
                    class:active={minViewsChoice === 1000}
                    onclick={() => (minViewsChoice = 1000)}
                  >
                    1k+
                  </button>
                  <button
                    type="button"
                    class="pill-choice"
                    class:active={minViewsChoice === 5000}
                    onclick={() => (minViewsChoice = 5000)}
                  >
                    5k+
                  </button>
                  <button
                    type="button"
                    class="pill-choice"
                    class:active={minViewsChoice === 20000}
                    onclick={() => (minViewsChoice = 20000)}
                  >
                    20k+
                  </button>
                  <button
                    type="button"
                    class="pill-choice"
                    class:active={minViewsChoice === 100000}
                    onclick={() => (minViewsChoice = 100000)}
                  >
                    100k+
                  </button>
                </div>
              </div>

              <div class="filter-box">
                <span class="filter-label">📅 {currentLang === 'bg' ? 'Период (дата):' : 'Recency window:'}</span>
                <div class="pill-group">
                  <button
                    type="button"
                    class="pill-choice"
                    class:active={recencyDays === 1}
                    onclick={() => (recencyDays = 1)}
                  >
                    24ч
                  </button>
                  <button
                    type="button"
                    class="pill-choice"
                    class:active={recencyDays === 3}
                    onclick={() => (recencyDays = 3)}
                  >
                    3 дни
                  </button>
                  <button
                    type="button"
                    class="pill-choice"
                    class:active={recencyDays === 7}
                    onclick={() => (recencyDays = 7)}
                  >
                    7 дни
                  </button>
                  <button
                    type="button"
                    class="pill-choice"
                    class:active={recencyDays === 30}
                    onclick={() => (recencyDays = 30)}
                  >
                    30 дни
                  </button>
                  <button
                    type="button"
                    class="pill-choice"
                    class:active={recencyDays === 0}
                    onclick={() => (recencyDays = 0)}
                  >
                    {currentLang === 'bg' ? 'Всички' : 'All'}
                  </button>
                </div>
              </div>

              {#if targetMode === 'watchlist'}
                <div class="filter-box">
                  <span class="filter-label">🎯 {currentLang === 'bg' ? 'Лимит канали:' : 'Target limit:'}</span>
                  <div class="pill-group">
                    <button
                      type="button"
                      class="pill-choice"
                      class:active={maxTargetsChoice === 5}
                      onclick={() => (maxTargetsChoice = 5)}
                    >
                      5 {currentLang === 'bg' ? 'канала' : 'ch'}
                    </button>
                    <button
                      type="button"
                      class="pill-choice"
                      class:active={maxTargetsChoice === 10}
                      onclick={() => (maxTargetsChoice = 10)}
                    >
                      10 {currentLang === 'bg' ? 'канала' : 'ch'}
                    </button>
                    <button
                      type="button"
                      class="pill-choice"
                      class:active={maxTargetsChoice === 0}
                      onclick={() => (maxTargetsChoice = 0)}
                    >
                      {currentLang === 'bg' ? 'Всички' : 'All'}
                    </button>
                  </div>
                </div>
              {/if}

              <div class="filter-box">
                <span class="filter-label">🏷️ {currentLang === 'bg' ? 'Хаштаг / Дума филтър:' : 'Hashtag / Keyword filter:'}</span>
                <div class="pill-group">
                  <button
                    type="button"
                    class="pill-choice"
                    class:active={allowNoKeywords}
                    onclick={() => {
                      allowNoKeywords = true;
                    }}
                  >
                    🔓 {currentLang === 'bg' ? 'Без филтър' : 'No filter'}
                  </button>
                  <button
                    type="button"
                    class="pill-choice"
                    class:active={!allowNoKeywords}
                    onclick={() => {
                      allowNoKeywords = false;
                    }}
                  >
                    🎯 {currentLang === 'bg' ? 'С филтър' : 'With filter'}
                  </button>
                </div>
              </div>
            </div>

            <div class="auto-process-row">
              <label class="checkbox-container">
                <input type="checkbox" bind:checked={autoProcess} />
                <div class="checkbox-label-block">
                  <span class="cb-title">⚙️ {currentLang === 'bg' ? 'Автоматичен пълен AI конвейер' : 'Auto-run full AI pipeline'}</span>
                  <span class="cb-desc">
                    {currentLang === 'bg'
                      ? 'Новооткритите видеа незабавно преминават през сваляне, транскрибиране, OCR и проверка на твърденията'
                      : 'Discovered videos will immediately queue for acquisition, transcription, OCR, and fact verification'}
                  </span>
                </div>
              </label>
            </div>
          </div>
        </div>

        {#if errorMessage}
          <div class="error-banner">
            ⚠️ {errorMessage}
          </div>
        {/if}

        {#if isScanning}
          <div class="scanning-indicator">
            <div class="spinner"></div>
            <div class="scan-status-text">
              <strong>{currentLang === 'bg' ? 'Сканиране на TikTok профили...' : 'Scanning TikTok channels...'}</strong>
              <span>
                {currentLang === 'bg' ? 'Изминало време:' : 'Elapsed time:'} {elapsedSec}s
              </span>
            </div>
          </div>
        {/if}

        {#if scanResult}
          <div class="results-box">
            <h4 class="results-title">
              ✓ {currentLang === 'bg' ? 'Резултати от сканирането' : 'Scan Results'}
            </h4>
            <div class="metrics-grid">
              <div class="metric-card">
                <span class="metric-val">{scanResult.targets_scanned}</span>
                <span class="metric-lbl">{currentLang === 'bg' ? 'Сканирани канала' : 'Channels scanned'}</span>
              </div>
              <div class="metric-card">
                <span class="metric-val">{scanResult.videos_found}</span>
                <span class="metric-lbl">{currentLang === 'bg' ? 'Намерени видеа' : 'Videos found'}</span>
              </div>
              <div class="metric-card highlight">
                <span class="metric-val green">+{scanResult.videos_ingested}</span>
                <span class="metric-lbl">{currentLang === 'bg' ? 'Новодобавени лийда' : 'Newly ingested'}</span>
              </div>
              <div class="metric-card">
                <span class="metric-val">{scanResult.videos_skipped_existing}</span>
                <span class="metric-lbl">{currentLang === 'bg' ? 'Вече в системата' : 'Already in DB'}</span>
              </div>
              <div class="metric-card">
                <span class="metric-val">{scanResult.videos_skipped_filter}</span>
                <span class="metric-lbl">{currentLang === 'bg' ? 'Под прага / филтри' : 'Filtered out'}</span>
              </div>
            </div>

            {#if scanResult.ingested_videos && scanResult.ingested_videos.length > 0}
              <div class="ingested-list-container">
                <span class="ingested-heading">
                  📥 {currentLang === 'bg' ? 'Новодобавени видеа в конвейера:' : 'Newly ingested videos in queue:'}
                </span>
                <div class="ingested-items">
                  {#each scanResult.ingested_videos as item}
                    <div class="ingested-item-card">
                      <div class="item-meta">
                        <span class="creator-badge">@{item.creator_handle || 'tiktok'}</span>
                        {#if item.views}
                          <span class="views-badge">👁️ {formatViews(item.views)}</span>
                        {/if}
                        {#if item.likes}
                          <span class="likes-badge">❤️ {formatViews(item.likes)}</span>
                        {/if}
                      </div>
                      <p class="item-caption">{item.caption || (currentLang === 'bg' ? '(Без описание)' : '(No caption)')}</p>
                      <div class="item-actions">
                        <button class="btn btn-sm btn-secondary" onclick={() => handleJump(item.id)}>
                          👁️ {currentLang === 'bg' ? 'Прегледай във входящата кутия' : 'View in Inbox'}
                        </button>
                      </div>
                    </div>
                  {/each}
                </div>
              </div>
            {:else}
              <div class="empty-ingested-notice">
                {currentLang === 'bg'
                  ? 'Няма намерени нови видеа над зададените прагове. Всички намерени видеа вече са налични в системата или не покриват филтрите.'
                  : 'No new videos above thresholds. All found videos are already in the database or filtered.'}
              </div>
            {/if}
          </div>
        {/if}
      </div>

      <div class="modal-footer">
        <button class="btn btn-secondary" onclick={onClose} disabled={isScanning}>
          {currentLang === 'bg' ? 'Затвори' : 'Close'}
        </button>

        <button
          class="btn btn-primary start-scan-btn"
          onclick={handleStartScan}
          disabled={isScanning}
        >
          {#if isScanning}
            <span class="btn-spinner"></span>
            {currentLang === 'bg' ? 'Сканиране в ход...' : 'Scanning...'}
          {:else}
            🚀 {currentLang === 'bg' ? 'Стартирай сканиране на TikTok' : 'Start TikTok Scan'}
          {/if}
        </button>
      </div>
    </div>
  </div>
{/if}

<style>
  .modal-backdrop {
    position: fixed;
    inset: 0;
    background: rgba(0, 0, 0, 0.7);
    display: flex;
    align-items: center;
    justify-content: center;
    z-index: 1050;
    backdrop-filter: blur(4px);
    padding: 1rem;
  }

  .scan-modal {
    background: var(--surface-bg);
    border: 1px solid var(--border-color);
    border-radius: 12px;
    width: 860px;
    max-width: 95vw;
    max-height: 90vh;
    display: flex;
    flex-direction: column;
    box-shadow: 0 16px 40px rgba(0, 0, 0, 0.35);
    overflow: hidden;
  }

  .modal-head {
    display: flex;
    justify-content: space-between;
    align-items: flex-start;
    padding: 1rem 1.25rem;
    border-bottom: 1px solid var(--border-color);
    background: var(--card-bg);
  }

  .title-row {
    display: flex;
    align-items: center;
    gap: 0.6rem;
    flex-wrap: wrap;
  }

  .title-row h3 {
    margin: 0;
    font-size: 1.15rem;
    font-weight: 700;
    color: var(--text-heading);
  }

  .scan-mode-badge,
  .scan-wl-badge {
    font-size: 0.75rem;
    padding: 0.2rem 0.55rem;
    border-radius: 999px;
    background: var(--pill-bg);
    border: 1px solid var(--border-color);
    color: var(--text-body);
    font-weight: 600;
  }

  .scan-sub {
    font-size: 0.8rem;
    color: var(--text-muted);
    margin-top: 0.25rem;
    display: block;
  }

  .close-btn {
    background: none;
    border: none;
    color: var(--text-muted);
    font-size: 1.2rem;
    cursor: pointer;
    padding: 0.2rem 0.4rem;
    border-radius: 4px;
    transition: all 0.15s ease;
  }

  .close-btn:hover {
    color: var(--text-heading);
    background: var(--hover-bg);
  }

  .modal-body {
    padding: 1.25rem;
    overflow-y: auto;
    display: flex;
    flex-direction: column;
    gap: 1.25rem;
    flex: 1;
  }

  .presets-bar {
    display: flex;
    align-items: center;
    gap: 0.5rem;
    flex-wrap: wrap;
    background: var(--card-bg);
    padding: 0.5rem 0.75rem;
    border-radius: 8px;
    border: 1px solid var(--border-color);
  }

  .presets-label {
    font-size: 0.8rem;
    font-weight: 700;
    color: var(--text-muted);
  }

  .preset-btn {
    background: var(--surface-bg);
    border: 1px solid var(--border-color);
    color: var(--text-body);
    font-size: 0.78rem;
    font-weight: 600;
    padding: 0.3rem 0.65rem;
    border-radius: 6px;
    cursor: pointer;
    transition: all 0.15s ease;
  }

  .preset-btn:hover {
    background: var(--hover-bg);
    border-color: var(--primary-accent);
    color: var(--primary-accent);
  }

  .scan-sections {
    display: flex;
    flex-direction: column;
    gap: 1rem;
  }

  .scan-section {
    background: var(--card-bg);
    border: 1px solid var(--border-color);
    border-radius: 8px;
    padding: 1rem;
    display: flex;
    flex-direction: column;
    gap: 0.75rem;
  }

  .section-title {
    margin: 0;
    font-size: 0.92rem;
    font-weight: 700;
    color: var(--text-heading);
  }

  .target-mode-selector {
    display: grid;
    grid-template-columns: repeat(auto-fit, minmax(220px, 1fr));
    gap: 0.65rem;
  }

  .mode-card {
    display: flex;
    align-items: flex-start;
    gap: 0.65rem;
    padding: 0.75rem;
    border: 1px solid var(--border-color);
    border-radius: 8px;
    background: var(--surface-bg);
    cursor: pointer;
    transition: all 0.15s ease;
  }

  .mode-card:hover {
    border-color: var(--primary-accent);
  }

  .mode-card.active {
    border-color: var(--primary-accent);
    background: rgba(99, 102, 241, 0.08);
  }

  .mode-info {
    display: flex;
    flex-direction: column;
    gap: 0.2rem;
  }

  .mode-name {
    font-size: 0.85rem;
    font-weight: 700;
    color: var(--text-heading);
  }

  .mode-desc {
    font-size: 0.75rem;
    color: var(--text-muted);
    line-height: 1.3;
  }

  .input-subgroup {
    display: flex;
    flex-direction: column;
    gap: 0.35rem;
    margin-top: 0.25rem;
  }

  .field-label-row {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 0.5rem;
  }

  .field-label {
    font-size: 0.8rem;
    font-weight: 600;
    color: var(--text-body);
  }

  .toggle-pill-label {
    display: inline-flex;
    align-items: center;
    gap: 0.35rem;
    font-size: 0.76rem;
    color: var(--primary-accent);
    cursor: pointer;
    font-weight: 600;
  }

  .field-hint {
    font-size: 0.72rem;
    color: var(--text-muted);
    line-height: 1.3;
  }

  .custom-textarea,
  .custom-input {
    width: 100%;
    padding: 0.55rem 0.75rem;
    background: var(--surface-bg);
    border: 1px solid var(--border-color);
    border-radius: 6px;
    color: var(--text-body);
    font-size: 0.85rem;
    box-sizing: border-box;
    font-family: inherit;
  }

  .custom-textarea:focus,
  .custom-input:focus {
    outline: none;
    border-color: var(--primary-accent);
  }

  .filters-grid {
    display: grid;
    grid-template-columns: repeat(auto-fit, minmax(180px, 1fr));
    gap: 0.85rem;
  }

  .filter-box {
    display: flex;
    flex-direction: column;
    gap: 0.35rem;
  }

  .filter-label {
    font-size: 0.78rem;
    font-weight: 600;
    color: var(--text-muted);
  }

  .pill-group {
    display: flex;
    flex-wrap: wrap;
    gap: 0.3rem;
  }

  .pill-choice {
    background: var(--surface-bg);
    border: 1px solid var(--border-color);
    color: var(--text-body);
    font-size: 0.75rem;
    font-weight: 600;
    padding: 0.25rem 0.55rem;
    border-radius: 4px;
    cursor: pointer;
    transition: all 0.15s ease;
  }

  .pill-choice:hover {
    background: var(--hover-bg);
  }

  .pill-choice.active {
    background: var(--primary-accent);
    color: #fff;
    border-color: var(--primary-accent);
  }

  .auto-process-row {
    border-top: 1px solid var(--border-color);
    padding-top: 0.75rem;
    margin-top: 0.25rem;
  }

  .checkbox-container {
    display: flex;
    align-items: flex-start;
    gap: 0.65rem;
    cursor: pointer;
  }

  .checkbox-label-block {
    display: flex;
    flex-direction: column;
    gap: 0.15rem;
  }

  .cb-title {
    font-size: 0.85rem;
    font-weight: 700;
    color: var(--text-heading);
  }

  .cb-desc {
    font-size: 0.75rem;
    color: var(--text-muted);
  }

  .error-banner {
    padding: 0.65rem 0.9rem;
    background: rgba(239, 68, 68, 0.15);
    border: 1px solid rgba(239, 68, 68, 0.4);
    border-radius: 6px;
    color: #f87171;
    font-size: 0.85rem;
    font-weight: 600;
  }

  .scanning-indicator {
    display: flex;
    align-items: center;
    gap: 1rem;
    padding: 1rem;
    background: rgba(99, 102, 241, 0.1);
    border: 1px solid rgba(99, 102, 241, 0.3);
    border-radius: 8px;
  }

  .spinner {
    width: 24px;
    height: 24px;
    border: 3px solid rgba(99, 102, 241, 0.3);
    border-top-color: var(--primary-accent);
    border-radius: 50%;
    animation: spin 0.8s linear infinite;
  }

  .btn-spinner {
    display: inline-block;
    width: 14px;
    height: 14px;
    border: 2px solid rgba(255, 255, 255, 0.3);
    border-top-color: #fff;
    border-radius: 50%;
    animation: spin 0.8s linear infinite;
    vertical-align: middle;
    margin-right: 0.35rem;
  }

  @keyframes spin {
    to {
      transform: rotate(360deg);
    }
  }

  .scan-status-text {
    display: flex;
    flex-direction: column;
    gap: 0.2rem;
    font-size: 0.85rem;
    color: var(--text-body);
  }

  .results-box {
    background: var(--card-bg);
    border: 1px solid var(--border-color);
    border-radius: 8px;
    padding: 1rem;
    display: flex;
    flex-direction: column;
    gap: 0.85rem;
  }

  .results-title {
    margin: 0;
    font-size: 0.95rem;
    font-weight: 700;
    color: var(--text-heading);
  }

  .metrics-grid {
    display: grid;
    grid-template-columns: repeat(auto-fit, minmax(130px, 1fr));
    gap: 0.65rem;
  }

  .metric-card {
    background: var(--surface-bg);
    border: 1px solid var(--border-color);
    border-radius: 6px;
    padding: 0.65rem;
    display: flex;
    flex-direction: column;
    align-items: center;
    gap: 0.2rem;
  }

  .metric-card.highlight {
    border-color: rgba(34, 197, 94, 0.4);
    background: rgba(34, 197, 94, 0.08);
  }

  .metric-val {
    font-size: 1.3rem;
    font-weight: 800;
    color: var(--text-heading);
  }

  .metric-val.green {
    color: #22c55e;
  }

  .metric-lbl {
    font-size: 0.72rem;
    color: var(--text-muted);
    text-align: center;
  }

  .ingested-list-container {
    display: flex;
    flex-direction: column;
    gap: 0.5rem;
    margin-top: 0.5rem;
  }

  .ingested-heading {
    font-size: 0.82rem;
    font-weight: 700;
    color: var(--text-heading);
  }

  .ingested-items {
    display: flex;
    flex-direction: column;
    gap: 0.5rem;
    max-height: 220px;
    overflow-y: auto;
  }

  .ingested-item-card {
    background: var(--surface-bg);
    border: 1px solid var(--border-color);
    border-radius: 6px;
    padding: 0.65rem 0.85rem;
    display: flex;
    flex-direction: column;
    gap: 0.35rem;
  }

  .item-meta {
    display: flex;
    align-items: center;
    gap: 0.5rem;
  }

  .creator-badge {
    font-size: 0.78rem;
    font-weight: 700;
    color: var(--primary-accent);
  }

  .views-badge,
  .likes-badge {
    font-size: 0.72rem;
    padding: 0.15rem 0.45rem;
    border-radius: 4px;
    background: var(--pill-bg);
    color: var(--text-muted);
  }

  .item-caption {
    margin: 0;
    font-size: 0.8rem;
    color: var(--text-body);
    display: -webkit-box;
    -webkit-line-clamp: 2;
    line-clamp: 2;
    -webkit-box-orient: vertical;
    overflow: hidden;
  }

  .item-actions {
    display: flex;
    justify-content: flex-end;
  }

  .empty-ingested-notice {
    font-size: 0.82rem;
    color: var(--text-muted);
    font-style: italic;
    padding: 0.5rem 0;
  }

  .modal-footer {
    display: flex;
    justify-content: flex-end;
    align-items: center;
    gap: 0.75rem;
    padding: 0.85rem 1.25rem;
    border-top: 1px solid var(--border-color);
    background: var(--card-bg);
  }

  .start-scan-btn {
    font-weight: 700;
  }
</style>
