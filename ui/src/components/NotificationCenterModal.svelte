<script lang="ts">
  import { onMount } from 'svelte';
  import {
    fetchDedupProposals,
    applyDedupProposal,
    autoDedupPeople,
    fetchNotifications,
  } from '../lib/api';
  import {
    getPlaybackState,
    togglePlayback,
    seekPlayback,
    seekPlaybackRelative,
    setPlaybackVolume,
    togglePlaybackMute,
  } from '../lib/playback.svelte';
  import { getActiveAiJobs } from '../lib/aiTasks.svelte';
  import type { Language } from '../lib/i18n';
  import type { DedupProposal, SystemNotification, DedupSummary } from '../lib/types';

  let {
    isOpen,
    currentLang,
    onClose,
    onRefreshPeople,
    onOpenSubject,
    onJumpToVideo,
  }: {
    isOpen: boolean;
    currentLang: Language;
    onClose: () => void;
    onRefreshPeople?: () => void;
    onOpenSubject?: (subjectId: number) => void;
    onJumpToVideo?: (videoId: string) => void;
  } = $props();

  let activeTab = $state<'questions' | 'media' | 'notifications'>('questions');
  let proposals = $state<DedupProposal[]>([]);
  let notifications = $state<SystemNotification[]>([]);
  let isLoadingProposals = $state(false);
  let isApplying = $state(false);
  let isAutoDeduping = $state(false);
  let actionToast = $state<string | null>(null);
  let nowTick = $state(Date.now());
  let initialTabChosen = $state(false);

  const playback = $derived(getPlaybackState());
  const clientJobs = $derived(getActiveAiJobs());

  const ongoingItems = $derived.by<SystemNotification[]>(() => {
    const serverOngoing = notifications.filter((n) => n.type === 'ongoing');
    const merged: SystemNotification[] = [...serverOngoing];
    for (const cj of clientJobs) {
      const alreadyCovered = serverOngoing.some(
        (s) =>
          (cj.type === 'person-research' && s.subject_id != null && String(s.subject_id) === String(cj.targetId)) ||
          (cj.type === 'video-research' && s.video_id != null && String(s.video_id) === String(cj.targetId)) ||
          (cj.type === 'aegis-scan' && s.video_id != null && String(s.video_id) === String(cj.targetId))
      );
      if (!alreadyCovered) {
        const elapsedSec = Math.max(1, Math.floor((nowTick - cj.startedAt) / 1000));
        const kindTitle =
          cj.type === 'person-research'
            ? `Проучване на обект: ${cj.label}`
            : cj.type === 'creator-risk'
              ? `Проверка на автор: ${cj.label}`
              : cj.type === 'video-research'
                ? `Допълнителна проверка на видео: ${cj.label}`
                : `Обработка в ход: ${cj.label}`;
        merged.push({
          id: `client_${cj.id}`,
          type: 'ongoing',
          title: kindTitle,
          message: cj.stage || `Изпълнява се (${elapsedSec} сек.)`,
          timestamp: new Date(cj.startedAt).toISOString(),
          progress_pct: Math.min(92, 18 + elapsedSec * 3),
          subject_id: cj.type === 'person-research' ? Number(cj.targetId) : null,
          video_id: cj.type === 'video-research' || cj.type === 'aegis-scan' ? String(cj.targetId) : null,
        });
      }
    }
    return merged;
  });

  const regularNotifications = $derived(notifications.filter((n) => n.type !== 'ongoing'));
  const totalNotifBadge = $derived(ongoingItems.length + regularNotifications.length);

  onMount(() => {
    if (isOpen) {
      void loadData(true);
    }
  });

  $effect(() => {
    if (!isOpen) {
      initialTabChosen = false;
      return;
    }
    void loadData(!initialTabChosen);
    initialTabChosen = true;
    const poll = setInterval(() => {
      nowTick = Date.now();
      void loadData(false);
    }, 2200);
    return () => clearInterval(poll);
  });

  async function loadData(selectDefaultTab = false) {
    if (selectDefaultTab) {
      isLoadingProposals = true;
    }
    try {
      const [pList, nRes] = await Promise.all([
        fetchDedupProposals().catch(() => []),
        fetchNotifications().catch(() => ({ ok: true, unread_count: 0, questions_count: 0, notifications: [] })),
      ]);
      proposals = pList;
      notifications = nRes.notifications || [];
      if (selectDefaultTab) {
        const hasOngoing =
          (nRes.notifications || []).some((x) => x.type === 'ongoing') || clientJobs.length > 0;
        if (hasOngoing) {
          activeTab = 'notifications';
        } else if (proposals.length > 0) {
          activeTab = 'questions';
        } else {
          activeTab = 'notifications';
        }
      }
    } finally {
      isLoadingProposals = false;
    }
  }

  function formatElapsedOrClock(ts: string): string {
    const parsed = Date.parse(ts);
    if (!isNaN(parsed)) {
      const diffSec = Math.max(0, Math.floor((nowTick - parsed) / 1000));
      if (diffSec < 3600) {
        const m = Math.floor(diffSec / 60);
        const s = diffSec % 60;
        return m > 0 ? `${m}м ${s}с` : `${s}с`;
      }
      return new Date(parsed).toLocaleString(currentLang === 'bg' ? 'bg-BG' : 'en-US');
    }
    return ts;
  }

  function showToast(msg: string) {
    actionToast = msg;
    setTimeout(() => {
      if (actionToast === msg) {
        actionToast = null;
      }
    }, 4000);
  }

  async function handleApply(proposal: DedupProposal) {
    isApplying = true;
    try {
      await applyDedupProposal(proposal);
      proposals = proposals.filter((p) => p.id !== proposal.id);
      showToast(
        currentLang === 'bg'
          ? `Успешно приложено: ${proposal.source_name}`
          : `Successfully applied: ${proposal.source_name}`
      );
      if (onRefreshPeople) {
        onRefreshPeople();
      }
    } catch (err: any) {
      showToast(err?.message || 'Грешка при прилагане');
    } finally {
      isApplying = false;
    }
  }

  function handleDismiss(proposal: DedupProposal) {
    proposals = proposals.filter((p) => p.id !== proposal.id);
  }

  async function handleAutoDedupAll() {
    isAutoDeduping = true;
    try {
      const res = await autoDedupPeople();
      const s = res.summary;
      showToast(
        currentLang === 'bg'
          ? `Автоматично обработени: ${s.merged} обединени, ${s.split} разделени, ${s.tagged} коригирани.`
          : `Auto processed: ${s.merged} merged, ${s.split} split, ${s.tagged} cleaned.`
      );
      await loadData();
      if (onRefreshPeople) {
        onRefreshPeople();
      }
    } catch (err: any) {
      showToast(err?.message || 'Грешка при автоматичното обединяване');
    } finally {
      isAutoDeduping = false;
    }
  }

  function formatTime(sec: number): string {
    if (isNaN(sec) || sec < 0) return '0:00';
    const m = Math.floor(sec / 60);
    const s = Math.floor(sec % 60);
    return `${m}:${s < 10 ? '0' : ''}${s}`;
  }
</script>

{#if isOpen}
  <div
    class="modal-backdrop"
    role="presentation"
    onclick={(e) => {
      if (e.target === e.currentTarget) onClose();
    }}
    onkeydown={(e) => {
      if (e.key === 'Escape') onClose();
    }}
  >
    <div class="modal-card" role="dialog" aria-modal="true">
      <header class="modal-header">
        <div class="header-title-box">
          <span class="header-icon">🔔</span>
          <div>
            <h3>
              {currentLang === 'bg'
                ? 'Известия и възпроизвеждане'
                : 'Notifications & Playback'}
            </h3>
            <p class="header-sub">
              {currentLang === 'bg'
                ? 'Въпроси за дубликати, системни събития и управление на видеото'
                : 'Entity merge questions, events, and media playback'}
            </p>
          </div>
        </div>
        <button class="close-btn" onclick={onClose} aria-label="Затвори">✕</button>
      </header>

      {#if actionToast}
        <div class="toast-banner">
          <span>ℹ️ {actionToast}</span>
        </div>
      {/if}

      <div class="tabs-bar">
        <button
          class="tab-btn"
          class:active={activeTab === 'questions'}
          onclick={() => (activeTab = 'questions')}
        >
          <span>❓ {currentLang === 'bg' ? 'Въпроси и дубликати' : 'Questions & Dedup'}</span>
          {#if proposals.length > 0}
            <span class="tab-badge warning">{proposals.length}</span>
          {/if}
        </button>

        <button
          class="tab-btn"
          class:active={activeTab === 'media'}
          onclick={() => (activeTab = 'media')}
        >
          <span>🎬 {currentLang === 'bg' ? 'Възпроизвеждане' : 'Playback'}</span>
          {#if playback.isPlaying}
            <span class="tab-pulse-dot"></span>
          {/if}
        </button>

        <button
          class="tab-btn"
          class:active={activeTab === 'notifications'}
          onclick={() => (activeTab = 'notifications')}
        >
          <span>📢 {currentLang === 'bg' ? 'Известия и проучвания' : 'Activity & Reports'}</span>
          {#if ongoingItems.length > 0}
            <span class="tab-pulse-dot ongoing-dot"></span>
          {/if}
          {#if totalNotifBadge > 0}
            <span class="tab-badge info">{totalNotifBadge}</span>
          {/if}
        </button>
      </div>

      {#if ongoingItems.length > 0 && activeTab !== 'notifications'}
        <button
          type="button"
          class="ongoing-top-strip"
          onclick={() => (activeTab = 'notifications')}
        >
          <span class="ongoing-spin">⏳</span>
          <span>
            {currentLang === 'bg'
              ? `${ongoingItems.length} проучване в ход: ${ongoingItems[0].title} — натиснете за преглед`
              : `${ongoingItems.length} active research job: ${ongoingItems[0].title}`}
          </span>
        </button>
      {/if}

      <div class="modal-body">
        {#if activeTab === 'questions'}
          <div class="questions-section">
            <div class="section-toolbar">
              <div class="toolbar-info">
                <h4>
                  {currentLang === 'bg'
                    ? 'Открити дубликати и съставни имена'
                    : 'Discovered Duplicates & Compound Names'}
                </h4>
                <p>
                  {currentLang === 'bg'
                    ? 'Прегледайте и редактирайте имената преди обединяване.'
                    : 'Review and edit canonical names before merging.'}
                </p>
              </div>

              {#if proposals.length > 0}
                <button
                  class="btn btn-primary btn-auto"
                  onclick={handleAutoDedupAll}
                  disabled={isAutoDeduping}
                >
                  {#if isAutoDeduping}
                    <span class="spinner">⏳</span>
                  {:else}
                    <span>⚡</span>
                  {/if}
                  <span>
                    {currentLang === 'bg'
                      ? 'Обедини всички автоматично'
                      : 'Auto-merge all'}
                  </span>
                </button>
              {/if}
            </div>

            {#if isLoadingProposals}
              <div class="loading-box">
                <span class="spinner">🔄</span>
                <span>{currentLang === 'bg' ? 'Зареждане на предложения...' : 'Loading proposals...'}</span>
              </div>
            {:else if proposals.length === 0}
              <div class="empty-box">
                <span class="empty-icon">✅</span>
                <h4>{currentLang === 'bg' ? 'Няма чакащи въпроси' : 'No pending questions'}</h4>
                <p>
                  {currentLang === 'bg'
                    ? 'Всички лица в базата данни са проверени, категоризирани и обединени.'
                    : 'All entities are clean, classified, and deduplicated.'}
                </p>
              </div>
            {:else}
              <div class="proposals-list">
                {#each proposals as p (p.id)}
                  <div class="proposal-card">
                    <div class="proposal-header">
                      <div class="proposal-type-box">
                        {#if p.action_type === 'Merge' || p.action_type === 'merge'}
                          <span class="pill-type merge">🔄 {currentLang === 'bg' ? 'Обединяване' : 'Merge'}</span>
                        {:else if p.action_type === 'SplitCompound' || p.action_type === 'split'}
                          <span class="pill-type split">✂️ {currentLang === 'bg' ? 'Разделяне на 2 лица' : 'Split 2 people'}</span>
                        {:else}
                          <span class="pill-type tag">🏷️ {currentLang === 'bg' ? 'Премахване на таг' : 'Extract tag'}</span>
                        {/if}
                        <span class="pill-confidence">{Math.round(p.confidence * 100)}% {currentLang === 'bg' ? 'увереност' : 'confidence'}</span>
                      </div>
                      <span class="proposal-id">ID: {p.source_id}</span>
                    </div>

                    <h5 class="proposal-title">{p.title}</h5>
                    <p class="proposal-desc">{p.description}</p>

                    <div class="diff-box">
                      <div class="diff-col source">
                        <span class="diff-label">{currentLang === 'bg' ? 'Текущ запис' : 'Current entry'}:</span>
                        <span class="name-pill old-name">❌ {p.source_name}</span>
                      </div>
                      <span class="arrow-symbol">➔</span>
                      <div class="diff-col target">
                        <span class="diff-label">{currentLang === 'bg' ? 'Резултат' : 'Target'}:</span>
                        <div class="target-pills-row">
                          {#each p.target_names as tName, tIdx}
                            <div class="editable-name-box">
                              <span class="edit-prefix">✓</span>
                              <input
                                type="text"
                                class="target-name-input"
                                bind:value={p.target_names[tIdx]}
                                title="Редактирай каноничното име"
                              />
                            </div>
                          {/each}
                          {#if p.tag !== undefined && p.tag !== null}
                            <div class="editable-name-box editable-tag-box">
                              <span class="edit-prefix">🏷️ #</span>
                              <input
                                type="text"
                                class="target-name-input tag-input"
                                bind:value={p.tag}
                                title="Редактирай етикета"
                              />
                            </div>
                          {/if}
                        </div>
                      </div>
                    </div>

                    <div class="proposal-actions">
                      <button
                        class="btn btn-sm btn-outline"
                        onclick={() => handleDismiss(p)}
                        disabled={isApplying}
                      >
                        ✕ {currentLang === 'bg' ? 'Пропусни' : 'Dismiss'}
                      </button>
                      <button
                        class="btn btn-sm btn-success"
                        onclick={() => handleApply(p)}
                        disabled={isApplying}
                      >
                        ✓ {currentLang === 'bg' ? 'Одобри' : 'Approve'}
                      </button>
                    </div>
                  </div>
                {/each}
              </div>
            {/if}
          </div>
        {:else if activeTab === 'media'}
          <div class="media-section">
            {#if !playback.hasActiveVideo}
              <div class="empty-box">
                <span class="empty-icon">🎬</span>
                <h4>{currentLang === 'bg' ? 'Няма активно заредено видео' : 'No active video loaded'}</h4>
                <p>
                  {currentLang === 'bg'
                    ? 'Пуснете видео от входящата кутия или детайлния изглед, за да се появят медийните контроли тук.'
                    : 'Play a video from the inbox or detail drawer to control playback here.'}
                </p>
              </div>
            {:else}
              <div class="media-player-card">
                <div class="media-header">
                  <div class="media-meta">
                    <span class="creator-tag">@{playback.creatorHandle || 'tiktok_creator'}</span>
                    <h5 class="video-caption-preview">
                      {playback.caption || (currentLang === 'bg' ? 'Активно видео' : 'Active video')}
                    </h5>
                  </div>
                  <span class="state-badge" class:playing={playback.isPlaying}>
                    {playback.isPlaying
                      ? (currentLang === 'bg' ? '▶ Възпроизвежда се' : '▶ Playing')
                      : (currentLang === 'bg' ? '⏸ На пауза' : '⏸ Paused')}
                  </span>
                </div>

                <div class="scrubber-row">
                  <span class="time-label">{formatTime(playback.currentTime)}</span>
                  <input
                    type="range"
                    min="0"
                    max={playback.duration > 0 ? playback.duration : 100}
                    step="0.1"
                    value={playback.currentTime}
                    oninput={(e) => seekPlayback(Number(e.currentTarget.value))}
                    class="time-slider"
                  />
                  <span class="time-label">{formatTime(playback.duration)}</span>
                </div>

                <div class="controls-toolbar">
                  <div class="seek-buttons">
                    <button
                      class="btn btn-ctrl"
                      onclick={() => seekPlaybackRelative(-10)}
                      title="-10 сек"
                    >
                      ⏪ -10s
                    </button>
                    <button
                      class="btn btn-ctrl"
                      onclick={() => seekPlaybackRelative(-5)}
                      title="-5 сек"
                    >
                      ⏮ -5s
                    </button>
                  </div>

                  <button
                    class="btn btn-play-large"
                    onclick={togglePlayback}
                    title={playback.isPlaying ? 'Пауза' : 'Пусни'}
                  >
                    {playback.isPlaying ? '⏸' : '▶'}
                  </button>

                  <div class="seek-buttons">
                    <button
                      class="btn btn-ctrl"
                      onclick={() => seekPlaybackRelative(5)}
                      title="+5 сек"
                    >
                      +5s ⏭
                    </button>
                    <button
                      class="btn btn-ctrl"
                      onclick={() => seekPlaybackRelative(10)}
                      title="+10 сек"
                    >
                      +10s ⏩
                    </button>
                  </div>
                </div>

                <div class="volume-row">
                  <button
                    class="btn btn-icon-subtle"
                    onclick={togglePlaybackMute}
                    title={playback.isMuted ? 'Включи звук' : 'Заглуши'}
                  >
                    {playback.isMuted || playback.volume === 0 ? '🔇' : '🔊'}
                  </button>
                  <input
                    type="range"
                    min="0"
                    max="1"
                    step="0.05"
                    value={playback.isMuted ? 0 : playback.volume}
                    oninput={(e) => setPlaybackVolume(Number(e.currentTarget.value))}
                    class="volume-slider"
                  />
                  <span class="volume-label">
                    {playback.isMuted ? '0%' : `${Math.round(playback.volume * 100)}%`}
                  </span>
                </div>
              </div>
            {/if}
          </div>
        {:else if activeTab === 'notifications'}
          <div class="notifications-section">
            {#if ongoingItems.length > 0}
              <div class="ongoing-group">
                <div class="ongoing-group-head">
                  <span class="ongoing-live-dot"></span>
                  <h5>{currentLang === 'bg' ? 'Проучвания в ход' : 'Ongoing Research'} ({ongoingItems.length})</h5>
                </div>
                <div class="notifications-list">
                  {#each ongoingItems as task (task.id)}
                    <div class="notif-item ongoing">
                      <div class="notif-icon ongoing-spin">🔄</div>
                      <div class="notif-content">
                        <div class="notif-top">
                          <span class="notif-title">{task.title}</span>
                          <span class="notif-time">{formatElapsedOrClock(task.timestamp)}</span>
                        </div>
                        <p class="notif-msg">{task.message}</p>
                        <div class="ongoing-progress-track">
                          <div
                            class="ongoing-progress-fill"
                            style="width: {Math.max(12, Math.min(98, task.progress_pct ?? 45))}%"
                          ></div>
                        </div>
                      </div>
                    </div>
                  {/each}
                </div>
              </div>
            {/if}

            {#if ongoingItems.length === 0 && regularNotifications.length === 0}
              <div class="empty-box">
                <span class="empty-icon">🔔</span>
                <h4>{currentLang === 'bg' ? 'Няма нови известия' : 'No notifications'}</h4>
                <p>
                  {currentLang === 'bg'
                    ? 'Всички фонови проверки и аналитични справки са завършени.'
                    : 'All background checks and analytical reports are complete.'}
                </p>
              </div>
            {:else if regularNotifications.length > 0}
              {#if ongoingItems.length > 0}
                <h5 class="completed-group-title">{currentLang === 'bg' ? 'Завършени справки и известия' : 'Completed Reports & Alerts'}</h5>
              {/if}
              <div class="notifications-list">
                {#each regularNotifications as item (item.id)}
                  <div class="notif-item {item.type}">
                    <div class="notif-icon">
                      {#if item.type === 'question'}
                        ❓
                      {:else if item.type === 'success'}
                        📄
                      {:else if item.type === 'warning'}
                        ⚠️
                      {:else}
                        ℹ️
                      {/if}
                    </div>
                    <div class="notif-content">
                      <div class="notif-top">
                        <span class="notif-title">{item.title}</span>
                        <span class="notif-time">{item.timestamp}</span>
                      </div>
                      <p class="notif-msg">{item.message}</p>
                      {#if item.subject_id && onOpenSubject}
                        <div class="notif-item-actions">
                          <button
                            type="button"
                            class="btn btn-sm btn-open-report"
                            onclick={() => {
                              onOpenSubject?.(Number(item.subject_id));
                              onClose();
                            }}
                          >
                            📖 {currentLang === 'bg' ? 'Отвори справката' : 'Open Report'} ↗
                          </button>
                        </div>
                      {:else if item.video_id && onJumpToVideo}
                        <div class="notif-item-actions">
                          <button
                            type="button"
                            class="btn btn-sm btn-open-report"
                            onclick={() => {
                              onJumpToVideo?.(String(item.video_id));
                              onClose();
                            }}
                          >
                            🎬 {currentLang === 'bg' ? 'Към видеото' : 'Open Video'} ↗
                          </button>
                        </div>
                      {/if}
                    </div>
                  </div>
                {/each}
              </div>
            {/if}
          </div>
        {/if}
      </div>
    </div>
  </div>
{/if}

<style>
  .modal-backdrop {
    position: fixed;
    inset: 0;
    z-index: 9999;
    background: rgba(0, 0, 0, 0.72);
    backdrop-filter: blur(6px);
    display: flex;
    align-items: center;
    justify-content: center;
    padding: 1.5rem;
    animation: fadeIn 0.2s ease-out;
  }

  .modal-card {
    width: 100%;
    max-width: 760px;
    max-height: 86vh;
    background: var(--bg-surface, #1e232a);
    color: var(--text-primary, #f3f4f6);
    border-radius: 14px;
    border: 1px solid var(--border-color, #374151);
    box-shadow: 0 20px 48px rgba(0, 0, 0, 0.5);
    display: flex;
    flex-direction: column;
    overflow: hidden;
    animation: scaleIn 0.2s ease-out;
  }

  .modal-header {
    display: flex;
    align-items: center;
    justify-content: space-between;
    padding: 1.25rem 1.5rem;
    border-bottom: 1px solid var(--border-color, #374151);
    background: var(--bg-surface-elevated, #242a33);
  }

  .header-title-box {
    display: flex;
    align-items: center;
    gap: 0.75rem;
  }

  .header-icon {
    font-size: 1.6rem;
  }

  .modal-header h3 {
    margin: 0;
    font-size: 1.15rem;
    font-weight: 700;
  }

  .header-sub {
    margin: 0.15rem 0 0 0;
    font-size: 0.8rem;
    color: var(--text-secondary, #9ca3af);
  }

  .close-btn {
    background: transparent;
    border: none;
    font-size: 1.2rem;
    color: var(--text-secondary, #9ca3af);
    cursor: pointer;
    padding: 0.4rem;
    border-radius: 6px;
    transition: background 0.15s, color 0.15s;
  }

  .close-btn:hover {
    background: rgba(255, 255, 255, 0.08);
    color: var(--text-primary, #ffffff);
  }

  .toast-banner {
    padding: 0.65rem 1.25rem;
    background: #065f46;
    color: #ecfdf5;
    font-size: 0.85rem;
    font-weight: 600;
    border-bottom: 1px solid #047857;
  }

  .tabs-bar {
    display: flex;
    background: var(--bg-surface, #1e232a);
    border-bottom: 1px solid var(--border-color, #374151);
    padding: 0 1rem;
    gap: 0.5rem;
  }

  .tab-btn {
    display: flex;
    align-items: center;
    gap: 0.45rem;
    padding: 0.85rem 1rem;
    background: transparent;
    border: none;
    border-bottom: 2px solid transparent;
    color: var(--text-secondary, #9ca3af);
    font-size: 0.88rem;
    font-weight: 600;
    cursor: pointer;
    transition: color 0.15s, border-color 0.15s;
  }

  .tab-btn:hover {
    color: var(--text-primary, #ffffff);
  }

  .tab-btn.active {
    color: #3b82f6;
    border-bottom-color: #3b82f6;
  }

  .tab-badge {
    font-size: 0.72rem;
    padding: 0.15rem 0.45rem;
    border-radius: 999px;
    font-weight: 700;
  }

  .tab-badge.warning {
    background: #f59e0b;
    color: #1f2937;
  }

  .tab-badge.info {
    background: #3b82f6;
    color: #ffffff;
  }

  .tab-pulse-dot {
    width: 8px;
    height: 8px;
    border-radius: 50%;
    background: #10b981;
    box-shadow: 0 0 6px #10b981;
    animation: pulse 1.4s infinite;
  }

  .modal-body {
    padding: 1.25rem 1.5rem;
    overflow-y: auto;
    flex: 1;
  }

  .section-toolbar {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 1rem;
    margin-bottom: 1.2rem;
  }

  .toolbar-info h4 {
    margin: 0;
    font-size: 1rem;
    font-weight: 600;
  }

  .toolbar-info p {
    margin: 0.2rem 0 0 0;
    font-size: 0.82rem;
    color: var(--text-secondary, #9ca3af);
  }

  .btn {
    display: inline-flex;
    align-items: center;
    justify-content: center;
    gap: 0.4rem;
    padding: 0.5rem 0.9rem;
    font-size: 0.84rem;
    font-weight: 600;
    border-radius: 8px;
    cursor: pointer;
    border: none;
    transition: all 0.15s ease;
  }

  .btn:hover:not(:disabled) {
    filter: brightness(1.1);
  }

  .btn:disabled {
    opacity: 0.5;
    cursor: not-allowed;
  }

  .btn-primary {
    background: #3b82f6;
    color: #ffffff;
  }

  .btn-auto {
    white-space: nowrap;
    padding: 0.6rem 1rem;
  }

  .btn-sm {
    padding: 0.35rem 0.75rem;
    font-size: 0.8rem;
  }

  .btn-outline {
    background: rgba(255, 255, 255, 0.05);
    color: var(--text-secondary, #9ca3af);
    border: 1px solid var(--border-color, #4b5563);
  }

  .btn-outline:hover:not(:disabled) {
    background: rgba(255, 255, 255, 0.1);
    color: var(--text-primary, #ffffff);
  }

  .btn-success {
    background: #10b981;
    color: #ffffff;
  }

  .loading-box,
  .empty-box {
    display: flex;
    flex-direction: column;
    align-items: center;
    justify-content: center;
    padding: 3rem 1.5rem;
    text-align: center;
    gap: 0.5rem;
    color: var(--text-secondary, #9ca3af);
  }

  .empty-icon {
    font-size: 2.2rem;
    margin-bottom: 0.25rem;
  }

  .empty-box h4 {
    margin: 0;
    color: var(--text-primary, #ffffff);
    font-size: 1.05rem;
  }

  .empty-box p {
    margin: 0;
    font-size: 0.85rem;
    max-width: 440px;
  }

  .proposals-list {
    display: flex;
    flex-direction: column;
    gap: 1rem;
  }

  .proposal-card {
    background: var(--bg-surface-elevated, #252b36);
    border: 1px solid var(--border-color, #374151);
    border-radius: 10px;
    padding: 1.1rem;
    display: flex;
    flex-direction: column;
    gap: 0.65rem;
  }

  .proposal-header {
    display: flex;
    align-items: center;
    justify-content: space-between;
  }

  .proposal-type-box {
    display: flex;
    align-items: center;
    gap: 0.5rem;
  }

  .pill-type {
    font-size: 0.74rem;
    font-weight: 700;
    padding: 0.2rem 0.5rem;
    border-radius: 6px;
  }

  .pill-type.merge {
    background: rgba(59, 130, 246, 0.2);
    color: #60a5fa;
  }

  .pill-type.split {
    background: rgba(245, 158, 11, 0.2);
    color: #fbbf24;
  }

  .pill-type.tag {
    background: rgba(168, 85, 247, 0.2);
    color: #c084fc;
  }

  .pill-confidence {
    font-size: 0.72rem;
    color: #10b981;
    font-weight: 600;
  }

  .proposal-id {
    font-size: 0.72rem;
    color: var(--text-secondary, #6b7280);
  }

  .proposal-title {
    margin: 0;
    font-size: 0.95rem;
    font-weight: 600;
    color: var(--text-primary, #ffffff);
  }

  .proposal-desc {
    margin: 0;
    font-size: 0.83rem;
    color: var(--text-secondary, #9ca3af);
    line-height: 1.35;
  }

  .diff-box {
    display: flex;
    align-items: center;
    gap: 1rem;
    background: rgba(0, 0, 0, 0.25);
    padding: 0.65rem 0.9rem;
    border-radius: 8px;
    margin: 0.2rem 0;
    flex-wrap: wrap;
  }

  .diff-col {
    display: flex;
    align-items: center;
    gap: 0.5rem;
  }

  .diff-label {
    font-size: 0.75rem;
    color: var(--text-secondary, #9ca3af);
  }

  .arrow-symbol {
    color: #3b82f6;
    font-weight: bold;
    font-size: 1.1rem;
  }

  .target-pills-row {
    display: flex;
    align-items: center;
    gap: 0.4rem;
    flex-wrap: wrap;
  }

  .name-pill {
    font-size: 0.78rem;
    font-weight: 600;
    padding: 0.2rem 0.55rem;
    border-radius: 6px;
  }

  .old-name {
    background: rgba(239, 68, 68, 0.15);
    color: #f87171;
    border: 1px solid rgba(239, 68, 68, 0.3);
  }

  .new-name {
    background: rgba(16, 185, 129, 0.15);
    color: #34d399;
    border: 1px solid rgba(16, 185, 129, 0.3);
  }

  .tag-name {
    background: rgba(168, 85, 247, 0.15);
    color: #d8b4fe;
    border: 1px solid rgba(168, 85, 247, 0.3);
  }

  .editable-name-box {
    display: inline-flex;
    align-items: center;
    background: rgba(16, 185, 129, 0.15);
    border: 1px solid rgba(16, 185, 129, 0.4);
    border-radius: 6px;
    padding: 0.1rem 0.4rem;
    gap: 4px;
  }

  .edit-prefix {
    color: #10b981;
    font-size: 0.8rem;
    font-weight: 700;
  }

  .target-name-input {
    background: transparent;
    border: none;
    color: #34d399;
    font-size: 0.8rem;
    font-weight: 600;
    outline: none;
    min-width: 110px;
    padding: 2px 4px;
  }

  .target-name-input:focus {
    background: rgba(0, 0, 0, 0.3);
    border-radius: 4px;
  }

  .editable-tag-box {
    background: rgba(168, 85, 247, 0.15);
    border-color: rgba(168, 85, 247, 0.4);
  }

  .editable-tag-box .edit-prefix,
  .tag-input {
    color: #d8b4fe;
    min-width: 75px;
  }

  .proposal-actions {
    display: flex;
    justify-content: flex-end;
    gap: 0.5rem;
    margin-top: 0.3rem;
  }

  .media-player-card {
    background: var(--bg-surface-elevated, #252b36);
    border: 1px solid var(--border-color, #374151);
    border-radius: 12px;
    padding: 1.25rem;
    display: flex;
    flex-direction: column;
    gap: 1.1rem;
  }

  .media-header {
    display: flex;
    align-items: flex-start;
    justify-content: space-between;
    gap: 1rem;
  }

  .creator-tag {
    font-size: 0.8rem;
    font-weight: 700;
    color: #3b82f6;
  }

  .video-caption-preview {
    margin: 0.3rem 0 0 0;
    font-size: 0.9rem;
    font-weight: 500;
    line-height: 1.35;
    color: var(--text-primary, #ffffff);
    display: -webkit-box;
    -webkit-line-clamp: 2;
    line-clamp: 2;
    -webkit-box-orient: vertical;
    overflow: hidden;
  }

  .state-badge {
    font-size: 0.74rem;
    font-weight: 600;
    padding: 0.25rem 0.6rem;
    border-radius: 999px;
    background: rgba(255, 255, 255, 0.08);
    color: var(--text-secondary, #9ca3af);
    white-space: nowrap;
  }

  .state-badge.playing {
    background: rgba(16, 185, 129, 0.2);
    color: #34d399;
  }

  .scrubber-row {
    display: flex;
    align-items: center;
    gap: 0.75rem;
  }

  .time-label {
    font-size: 0.78rem;
    font-family: monospace;
    color: var(--text-secondary, #9ca3af);
    min-width: 38px;
  }

  .time-slider {
    flex: 1;
    accent-color: #3b82f6;
    cursor: pointer;
  }

  .controls-toolbar {
    display: flex;
    align-items: center;
    justify-content: center;
    gap: 1.5rem;
  }

  .seek-buttons {
    display: flex;
    gap: 0.5rem;
  }

  .btn-ctrl {
    background: rgba(255, 255, 255, 0.06);
    color: var(--text-primary, #ffffff);
    padding: 0.45rem 0.75rem;
    border-radius: 8px;
    font-size: 0.8rem;
  }

  .btn-play-large {
    width: 52px;
    height: 52px;
    border-radius: 50%;
    background: #3b82f6;
    color: #ffffff;
    font-size: 1.4rem;
    padding: 0;
    box-shadow: 0 4px 14px rgba(59, 130, 246, 0.4);
  }

  .volume-row {
    display: flex;
    align-items: center;
    gap: 0.75rem;
    padding-top: 0.5rem;
    border-top: 1px solid rgba(255, 255, 255, 0.06);
    max-width: 320px;
    margin: 0 auto;
    width: 100%;
  }

  .btn-icon-subtle {
    background: transparent;
    border: none;
    font-size: 1.1rem;
    cursor: pointer;
    padding: 0.2rem;
  }

  .volume-slider {
    flex: 1;
    accent-color: #3b82f6;
    cursor: pointer;
  }

  .volume-label {
    font-size: 0.76rem;
    font-family: monospace;
    color: var(--text-secondary, #9ca3af);
    min-width: 34px;
    text-align: right;
  }

  .notifications-list {
    display: flex;
    flex-direction: column;
    gap: 0.75rem;
  }

  .notif-item {
    display: flex;
    gap: 0.85rem;
    padding: 0.9rem 1rem;
    border-radius: 10px;
    background: var(--bg-surface-elevated, #252b36);
    border: 1px solid var(--border-color, #374151);
  }

  .notif-item.question {
    border-left: 4px solid #f59e0b;
  }

  .notif-item.success {
    border-left: 4px solid #10b981;
  }

  .notif-item.warning {
    border-left: 4px solid #ef4444;
  }

  .notif-item.info {
    border-left: 4px solid #3b82f6;
  }

  .notif-item.ongoing {
    border-left: 4px solid #06b6d4;
    background: linear-gradient(90deg, rgba(6, 182, 212, 0.12), rgba(37, 43, 54, 0.95));
  }

  .ongoing-group {
    margin-bottom: 1.1rem;
  }

  .ongoing-group-head {
    display: flex;
    align-items: center;
    gap: 0.5rem;
    margin-bottom: 0.55rem;
  }

  .ongoing-group-head h5,
  .completed-group-title {
    margin: 0 0 0.5rem 0;
    font-size: 0.8rem;
    text-transform: uppercase;
    letter-spacing: 0.04em;
    color: var(--text-secondary, #9ca3af);
  }

  .ongoing-live-dot {
    width: 8px;
    height: 8px;
    border-radius: 50%;
    background: #06b6d4;
    box-shadow: 0 0 8px #06b6d4;
    animation: pulse 1.2s infinite;
  }

  .ongoing-dot {
    background: #06b6d4 !important;
  }

  .ongoing-spin {
    display: inline-block;
    animation: spin360 1.8s linear infinite;
  }

  @keyframes spin360 {
    from {
      transform: rotate(0deg);
    }
    to {
      transform: rotate(360deg);
    }
  }

  .ongoing-progress-track {
    margin-top: 0.5rem;
    height: 5px;
    width: 100%;
    background: rgba(255, 255, 255, 0.08);
    border-radius: 999px;
    overflow: hidden;
  }

  .ongoing-progress-fill {
    height: 100%;
    background: linear-gradient(90deg, #06b6d4, #10b981);
    border-radius: 999px;
    transition: width 0.35s ease;
  }

  .ongoing-top-strip {
    display: flex;
    align-items: center;
    gap: 0.55rem;
    width: 100%;
    padding: 0.5rem 1.25rem;
    background: rgba(6, 182, 212, 0.14);
    border: none;
    border-bottom: 1px solid rgba(6, 182, 212, 0.3);
    color: #67e8f9;
    font-size: 0.8rem;
    font-weight: 600;
    cursor: pointer;
    text-align: left;
  }

  .notif-item-actions {
    margin-top: 0.5rem;
  }

  .btn-open-report {
    padding: 0.25rem 0.65rem;
    font-size: 0.76rem;
    font-weight: 600;
    border-radius: 6px;
    background: rgba(16, 185, 129, 0.15);
    border: 1px solid rgba(16, 185, 129, 0.38);
    color: #34d399;
    cursor: pointer;
  }

  .btn-open-report:hover {
    background: rgba(16, 185, 129, 0.26);
  }

  .notif-icon {
    font-size: 1.25rem;
  }

  .notif-content {
    flex: 1;
  }

  .notif-top {
    display: flex;
    align-items: center;
    justify-content: space-between;
    margin-bottom: 0.25rem;
  }

  .notif-title {
    font-size: 0.88rem;
    font-weight: 600;
    color: var(--text-primary, #ffffff);
  }

  .notif-time {
    font-size: 0.72rem;
    color: var(--text-secondary, #9ca3af);
  }

  .notif-msg {
    margin: 0;
    font-size: 0.82rem;
    color: var(--text-secondary, #9ca3af);
    line-height: 1.35;
  }

  @keyframes pulse {
    0%, 100% {
      transform: scale(1);
      opacity: 1;
    }
    50% {
      transform: scale(1.3);
      opacity: 0.6;
    }
  }

  @keyframes fadeIn {
    from {
      opacity: 0;
    }
    to {
      opacity: 1;
    }
  }

  @keyframes scaleIn {
    from {
      transform: scale(0.96);
      opacity: 0;
    }
    to {
      transform: scale(1);
      opacity: 1;
    }
  }
</style>
