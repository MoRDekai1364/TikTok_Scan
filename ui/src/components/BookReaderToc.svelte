<script lang="ts">
    import { onMount } from 'svelte';

    export interface TocItem {
        id: string;
        label: string;
        level?: 1 | 2 | 3;
        badge?: string;
        tone?: 'default' | 'green' | 'amber' | 'red' | 'blue';
    }

    let {
        items = [],
        scrollContainer = null,
        title = 'Съдържание',
        variant = 'sidebar',
    }: {
        items: TocItem[];
        scrollContainer?: HTMLElement | null;
        title?: string;
        variant?: 'sidebar' | 'compact';
    } = $props();

    let activeId = $state('');
    let progressPct = $state(0);
    let collapsed = $state(false);

    function computeScrollState() {
        if (!items.length) return;
        const container = scrollContainer;
        if (container) {
            const maxScroll = Math.max(1, container.scrollHeight - container.clientHeight);
            progressPct = Math.min(100, Math.max(0, Math.round((container.scrollTop / maxScroll) * 100)));
            const containerRect = container.getBoundingClientRect();
            const threshold = containerRect.top + Math.min(140, container.clientHeight * 0.28);
            let current = items[0]?.id ?? '';
            for (const item of items) {
                const el = container.querySelector(`#${CSS.escape(item.id)}`) ?? document.getElementById(item.id);
                if (el) {
                    const rect = el.getBoundingClientRect();
                    if (rect.top <= threshold) {
                        current = item.id;
                    }
                }
            }
            activeId = current;
        } else {
            const docEl = document.documentElement;
            const maxScroll = Math.max(1, docEl.scrollHeight - window.innerHeight);
            progressPct = Math.min(100, Math.max(0, Math.round((window.scrollY / maxScroll) * 100)));
            let current = items[0]?.id ?? '';
            for (const item of items) {
                const el = document.getElementById(item.id);
                if (el) {
                    const rect = el.getBoundingClientRect();
                    if (rect.top <= 160) {
                        current = item.id;
                    }
                }
            }
            activeId = current;
        }
    }

    function jumpTo(id: string) {
        activeId = id;
        const el =
            (scrollContainer ? scrollContainer.querySelector(`#${CSS.escape(id)}`) : null) ??
            document.getElementById(id);
        if (!el) return;
        if (scrollContainer) {
            const cRect = scrollContainer.getBoundingClientRect();
            const eRect = el.getBoundingClientRect();
            const targetTop = scrollContainer.scrollTop + (eRect.top - cRect.top) - 18;
            scrollContainer.scrollTo({ top: Math.max(0, targetTop), behavior: 'smooth' });
        } else {
            el.scrollIntoView({ behavior: 'smooth', block: 'start' });
        }
        el.classList.remove('toc-target-flash');
        void (el as HTMLElement).offsetWidth;
        el.classList.add('toc-target-flash');
        setTimeout(() => el.classList.remove('toc-target-flash'), 1400);
    }

    $effect(() => {
        const target: EventTarget = scrollContainer ?? window;
        computeScrollState();
        const handler = () => computeScrollState();
        target.addEventListener('scroll', handler, { passive: true });
        return () => target.removeEventListener('scroll', handler);
    });

    $effect(() => {
        if (items.length > 0 && !activeId) {
            activeId = items[0].id;
        }
    });

    onMount(() => {
        computeScrollState();
    });
</script>

{#if items.length > 0}
    {#if variant === 'compact'}
        <nav class="book-toc-compact" aria-label={title}>
            <div class="toc-compact-head">
                <span class="toc-compact-label">📑 {title}</span>
                <span class="toc-compact-pct">{progressPct}%</span>
            </div>
            <div class="toc-compact-pills">
                {#each items as item (item.id)}
                    <button
                        type="button"
                        class="toc-pill"
                        class:active={activeId === item.id}
                        class:tone-green={item.tone === 'green'}
                        class:tone-red={item.tone === 'red'}
                        class:tone-amber={item.tone === 'amber'}
                        onclick={() => jumpTo(item.id)}
                    >
                        <span>{item.label}</span>
                        {#if item.badge}
                            <span class="toc-pill-badge">{item.badge}</span>
                        {/if}
                    </button>
                {/each}
            </div>
        </nav>
    {:else}
        <aside class="book-toc-sidebar" class:collapsed aria-label={title}>
            <div class="toc-sidebar-header">
                <button
                    type="button"
                    class="toc-collapse-btn"
                    onclick={() => (collapsed = !collapsed)}
                    title={collapsed ? 'Разгъни съдържанието' : 'Свий съдържанието'}
                >
                    <span>📑</span>
                    {#if !collapsed}
                        <span class="toc-title-txt">{title}</span>
                    {/if}
                </button>
                {#if !collapsed}
                    <span class="toc-progress-txt">{progressPct}%</span>
                {/if}
            </div>

            {#if !collapsed}
                <div class="toc-progress-track">
                    <div class="toc-progress-fill" style="width: {progressPct}%"></div>
                </div>

                <ul class="toc-list">
                    {#each items as item (item.id)}
                        <li>
                            <button
                                type="button"
                                class="toc-entry"
                                class:active={activeId === item.id}
                                class:lvl-2={item.level === 2}
                                class:lvl-3={item.level === 3}
                                onclick={() => jumpTo(item.id)}
                            >
                                <span class="toc-entry-text">{item.label}</span>
                                {#if item.badge}
                                    <span class="toc-entry-badge">{item.badge}</span>
                                {/if}
                            </button>
                        </li>
                    {/each}
                </ul>
            {/if}
        </aside>
    {/if}
{/if}

<style>
    .book-toc-sidebar {
        width: var(--dossier-toc-w, 248px);
        min-width: var(--dossier-toc-w, 248px);
        max-height: 100%;
        display: flex;
        flex-direction: column;
        background: var(--bg-surface);
        border: 1px solid var(--border);
        border-radius: 10px;
        padding: 10px;
        gap: 8px;
        position: sticky;
        top: 0;
        align-self: flex-start;
        transition: width 0.18s ease, min-width 0.18s ease;
    }

    .book-toc-sidebar.collapsed {
        width: 44px;
        min-width: 44px;
        padding: 8px 6px;
        align-items: center;
    }

    .toc-sidebar-header {
        display: flex;
        align-items: center;
        justify-content: space-between;
        gap: 6px;
    }

    .toc-collapse-btn {
        display: inline-flex;
        align-items: center;
        gap: 6px;
        background: transparent;
        border: none;
        color: var(--text-primary);
        font-size: 12px;
        font-weight: 700;
        cursor: pointer;
        padding: 2px 4px;
        border-radius: 6px;
    }

    .toc-collapse-btn:hover {
        background: var(--bg-elevated);
    }

    .toc-title-txt {
        letter-spacing: 0.02em;
    }

    .toc-progress-txt {
        font-size: 11px;
        font-weight: 600;
        color: var(--text-muted);
        font-variant-numeric: tabular-nums;
    }

    .toc-progress-track {
        height: 4px;
        width: 100%;
        background: var(--bg-elevated);
        border-radius: 999px;
        overflow: hidden;
    }

    .toc-progress-fill {
        height: 100%;
        background: #10b981;
        border-radius: 999px;
        transition: width 0.15s ease-out;
    }

    .toc-list {
        list-style: none;
        margin: 0;
        padding: 0;
        display: flex;
        flex-direction: column;
        gap: 2px;
        overflow-y: auto;
        max-height: calc(var(--dossier-max-h, 92vh) - 155px);
    }

    .toc-entry {
        width: 100%;
        text-align: left;
        display: flex;
        align-items: center;
        justify-content: space-between;
        gap: 6px;
        padding: 6px 8px;
        border: none;
        border-left: 2px solid transparent;
        border-radius: 0 6px 6px 0;
        background: transparent;
        color: var(--text-secondary);
        font-size: 12px;
        font-weight: 600;
        line-height: 1.3;
        cursor: pointer;
        transition: background 0.14s ease, color 0.14s ease, border-color 0.14s ease;
    }

    .toc-entry.lvl-2 {
        padding-left: 16px;
        font-size: 11.5px;
        font-weight: 500;
    }

    .toc-entry.lvl-3 {
        padding-left: 24px;
        font-size: 11px;
        font-weight: 400;
        color: var(--text-muted);
    }

    .toc-entry:hover {
        background: var(--bg-elevated);
        color: var(--text-primary);
    }

    .toc-entry.active {
        background: rgba(16, 185, 129, 0.12);
        border-left-color: #10b981;
        color: var(--text-primary);
        font-weight: 700;
    }

    .toc-entry-text {
        overflow: hidden;
        text-overflow: ellipsis;
        display: -webkit-box;
        line-clamp: 2;
        -webkit-line-clamp: 2;
        line-clamp: 2;
        -webkit-box-orient: vertical;
    }

    .toc-entry-badge {
        font-size: 10px;
        padding: 1px 5px;
        border-radius: 999px;
        background: var(--bg-elevated);
        color: var(--text-muted);
        flex-shrink: 0;
    }

    .book-toc-compact {
        display: flex;
        align-items: center;
        gap: 8px;
        padding: 6px 10px;
        background: var(--bg-surface);
        border: 1px solid var(--border);
        border-radius: 8px;
        flex-wrap: wrap;
    }

    .toc-compact-head {
        display: inline-flex;
        align-items: center;
        gap: 6px;
        padding-right: 6px;
        border-right: 1px solid var(--border);
    }

    .toc-compact-label {
        font-size: 11.5px;
        font-weight: 700;
        color: var(--text-secondary);
        white-space: nowrap;
    }

    .toc-compact-pct {
        font-size: 10.5px;
        font-weight: 600;
        color: #10b981;
        font-variant-numeric: tabular-nums;
    }

    .toc-compact-pills {
        display: flex;
        align-items: center;
        gap: 5px;
        flex-wrap: wrap;
        flex: 1;
    }

    .toc-pill {
        display: inline-flex;
        align-items: center;
        gap: 4px;
        padding: 3px 8px;
        border-radius: 999px;
        border: 1px solid var(--border);
        background: var(--bg-elevated);
        color: var(--text-secondary);
        font-size: 11px;
        font-weight: 500;
        cursor: pointer;
        transition: all 0.14s ease;
    }

    .toc-pill:hover {
        border-color: #10b981;
        color: var(--text-primary);
    }

    .toc-pill.active {
        background: rgba(16, 185, 129, 0.15);
        border-color: #10b981;
        color: var(--text-primary);
        font-weight: 700;
    }

    .toc-pill.tone-green {
        border-color: rgba(16, 185, 129, 0.45);
    }

    .toc-pill.tone-red {
        border-color: rgba(239, 68, 68, 0.45);
    }

    .toc-pill.tone-amber {
        border-color: rgba(245, 158, 11, 0.45);
    }

    .toc-pill-badge {
        font-size: 10px;
        opacity: 0.85;
    }

    @media (max-width: 900px) {
        .book-toc-sidebar {
            width: 100%;
            min-width: 0;
            position: static;
        }
        .toc-list {
            max-height: 180px;
        }
    }
</style>
