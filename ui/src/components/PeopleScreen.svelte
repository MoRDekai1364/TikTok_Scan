<script lang="ts">
  import { onMount } from 'svelte';
  import type { Language } from '../lib/i18n';
  import { translations } from '../lib/i18n';
  import type { Person } from '../lib/types';
  import PersonDossierModal from './PersonDossierModal.svelte';
  import {
    fetchPeople,
    createPerson,
    updatePerson,
    deletePerson,
    researchPerson,
    autoClassifyPeople,
    autoDedupPeople,
    mergePeopleDirectly,
  } from '../lib/api';
  import { isJobActive, startAiJob, finishAiJob } from '../lib/aiTasks.svelte';

  let { currentLang }: { currentLang: Language } = $props();
  const t = $derived(translations[currentLang]);

  let people = $state<Person[]>([]);
  let isLoading = $state(false);
  let isClassifying = $state(false);
  let searchQuery = $state('');
  let selectedCategory = $state('all');
  let selectedEntityType = $state('all');
  let selectedSort = $state('rating');
  let toastMsg = $state<string | null>(null);

  let selectedPeopleIds = $state<number[]>([]);
  let isMergeModalOpen = $state(false);
  let mergeMasterId = $state<number | null>(null);
  let mergeCustomName = $state('');
  let isMerging = $state(false);

  let researchingId = $state<number | null>(null);
  let editingPerson = $state<Person | null>(null);
  let viewingDossier = $state<Person | null>(null);
  let isCreatingNew = $state(false);

  let newName = $state('');
  let newRole = $state('');
  let newCategory = $state('Политик');
  let newEntityType = $state('person');
  let newRating = $state(70);
  let newTagsStr = $state('');
  let newNotes = $state('');

  const ENTITY_TYPES = [
    { id: 'all', bg: 'Всички видове обекти', en: 'All Entity Types', icon: '🌐' },
    { id: 'person', bg: 'Личности', en: 'People', icon: '👤' },
    { id: 'institution', bg: 'Институции и органи', en: 'Institutions', icon: '🏛️' },
    { id: 'infrastructure', bg: 'Инфраструктура (пътища, мостове, мрежи)', en: 'Infrastructure', icon: '🛤️' },
    { id: 'building', bg: 'Сгради и обекти', en: 'Buildings & Facilities', icon: '🏢' },
    { id: 'group', bg: 'Организации и партии', en: 'Groups & Parties', icon: '👥' },
    { id: 'channel', bg: 'Канали и медии', en: 'Channels & Media', icon: '📺' },
    { id: 'country', bg: 'Държави', en: 'Countries', icon: '🌍' },
    { id: 'event', bg: 'Събития', en: 'Events', icon: '📅' },
  ];

  function getEntityTypeInfo(type?: string) {
    switch (type) {
      case 'institution':
        return { label: currentLang === 'bg' ? 'Институция' : 'Institution', icon: '🏛️', cssClass: 'type-institution' };
      case 'infrastructure':
        return { label: currentLang === 'bg' ? 'Инфраструктура' : 'Infrastructure', icon: '🛤️', cssClass: 'type-infrastructure' };
      case 'building':
        return { label: currentLang === 'bg' ? 'Сграда / Обект' : 'Building', icon: '🏢', cssClass: 'type-building' };
      case 'channel':
        return { label: currentLang === 'bg' ? 'Канал / Медия' : 'Channel / Media', icon: '📺', cssClass: 'type-channel' };
      case 'country':
        return { label: currentLang === 'bg' ? 'Държава' : 'Country', icon: '🌍', cssClass: 'type-country' };
      case 'group':
        return { label: currentLang === 'bg' ? 'Група / Партия' : 'Group / Party', icon: '👥', cssClass: 'type-group' };
      case 'event':
        return { label: currentLang === 'bg' ? 'Събитие' : 'Event', icon: '📅', cssClass: 'type-event' };
      default:
        return { label: currentLang === 'bg' ? 'Личност' : 'Person', icon: '👤', cssClass: 'type-person' };
    }
  }

  const CATEGORIES = [
    { id: 'all', bg: 'Всички категории', en: 'All Categories' },
    { id: 'Политик', bg: 'Политици', en: 'Politicians' },
    { id: 'Институция', bg: 'Държавни институции', en: 'State Institutions' },
    { id: 'Инфраструктура', bg: 'Инфраструктурни проекти', en: 'Infrastructure' },
    { id: 'Сграда / Обект', bg: 'Сгради и имоти', en: 'Buildings & Properties' },
    { id: 'Магистрат', bg: 'Съдии и прокурори', en: 'Judiciary' },
    { id: 'Бизнесмен', bg: 'Бизнес и инвеститори', en: 'Business' },
    { id: 'Медицинско лице', bg: 'Лекари и медици', en: 'Medics' },
    { id: 'Ключова фигура', bg: 'Ръководни длъжности', en: 'Key Positions' },
    { id: 'Общественик', bg: 'Общественици и НПО', en: 'Public Figures' },
    { id: 'Медийна личност', bg: 'Журналисти и водещи', en: 'Media' },
  ];

  function showToast(msg: string) {
    toastMsg = msg;
    setTimeout(() => {
      if (toastMsg === msg) toastMsg = null;
    }, 3000);
  }

  async function loadData() {
    isLoading = true;
    try {
      people = await fetchPeople({
        query: searchQuery || undefined,
        category: selectedCategory !== 'all' ? selectedCategory : undefined,
        sort_by: selectedSort,
        entity_type: selectedEntityType !== 'all' ? selectedEntityType : undefined,
      });
    } catch {
      showToast(currentLang === 'bg' ? 'Грешка при зареждане на списъка' : 'Error loading people');
    } finally {
      isLoading = false;
    }
  }

  onMount(() => {
    loadData();
  });

  async function handleSearch() {
    await loadData();
  }

  async function handleAutoClassify() {
    if (isClassifying) return;
    isClassifying = true;
    try {
      const res = await autoClassifyPeople();
      showToast(
        currentLang === 'bg'
          ? `Обновени: ${res.updated}, Премахнат шум: ${res.pruned} (общо ${res.total})`
          : `Updated: ${res.updated}, Pruned noise: ${res.pruned} (total ${res.total})`
      );
      await loadData();
    } catch {
      showToast(currentLang === 'bg' ? 'Грешка при авто-класификация' : 'Auto-classify failed');
    } finally {
      isClassifying = false;
    }
  }

  let isAutoDeduping = $state(false);

  async function handleAutoDedup() {
    if (isAutoDeduping) return;
    isAutoDeduping = true;
    try {
      const res = await autoDedupPeople();
      const s = res.summary;
      showToast(
        currentLang === 'bg'
          ? `Дедупликация: ${s.merged} обединени, ${s.split} разделени, ${s.tagged} коригирани.`
          : `Dedup: ${s.merged} merged, ${s.split} split, ${s.tagged} cleaned.`
      );
      await loadData();
    } catch {
      showToast(currentLang === 'bg' ? 'Грешка при дедупликацията' : 'Dedup failed');
    } finally {
      isAutoDeduping = false;
    }
  }

  async function handleTriggerResearch(p: Person) {
    if (isJobActive('person-research', p.id)) return;
    startAiJob('person-research', p.id, p.canonical_name);
    showToast(currentLang === 'bg' ? `⏳ Изготвяне на справка за ${p.canonical_name}...` : `⏳ Researching ${p.canonical_name}...`);
    try {
      const updated = await researchPerson(p.id, p.canonical_name);
      people = people.map((item) => (item.id === p.id ? { ...item, ...updated } : item));
      finishAiJob('person-research', p.id, currentLang === 'bg' ? `✓ Справката за ${p.canonical_name} е готова` : `✓ Report updated`, true);
      showToast(currentLang === 'bg' ? `✓ Справката за ${p.canonical_name} е обновена` : `✓ Report updated`);
    } catch {
      finishAiJob('person-research', p.id, currentLang === 'bg' ? 'Грешка при проучването' : 'Research error', false);
      showToast(currentLang === 'bg' ? 'Грешка при проучването' : 'Research error');
    }
  }

  async function handleQuickAddTag(p: Person) {
    const input = prompt(currentLang === 'bg' ? 'Въведете нов таг (напр. [десен], [министър], [НАТО]):' : 'Enter tag:');
    if (!input || !input.trim()) return;
    const tag = input.trim();
    const currTags = p.tags || [];
    if (currTags.includes(tag)) return;
    const nextTags = [...currTags, tag];
    try {
      await updatePerson(p.id, { tags: nextTags });
      p.tags = nextTags;
      showToast(currentLang === 'bg' ? '✓ Тагът е добавен!' : '✓ Tag added!');
    } catch {
      showToast(currentLang === 'bg' ? 'Грешка при запис на тага' : 'Tag update error');
    }
  }

  async function handleRemoveTag(p: Person, tagToRemove: string) {
    const nextTags = (p.tags || []).filter((t) => t !== tagToRemove);
    try {
      await updatePerson(p.id, { tags: nextTags });
      p.tags = nextTags;
    } catch {
      showToast(currentLang === 'bg' ? 'Грешка' : 'Error');
    }
  }

  async function handleRatingChange(p: Person, newR: number) {
    p.rating = newR;
    try {
      await updatePerson(p.id, { rating: newR });
    } catch {}
  }

  function togglePersonSelection(id: number) {
    if (selectedPeopleIds.includes(id)) {
      selectedPeopleIds = selectedPeopleIds.filter(x => x !== id);
    } else {
      selectedPeopleIds = [...selectedPeopleIds, id];
    }
  }

  function clearSelection() {
    selectedPeopleIds = [];
  }

  function openMergeModal() {
    if (selectedPeopleIds.length < 2) return;
    mergeMasterId = selectedPeopleIds[0];
    const master = people.find(p => p.id === mergeMasterId);
    mergeCustomName = master ? master.canonical_name : '';
    isMergeModalOpen = true;
  }

  async function handleExecuteMerge() {
    if (!mergeMasterId || selectedPeopleIds.length < 2) return;
    isMerging = true;
    try {
      const sourcesToMerge = selectedPeopleIds.filter(id => id !== mergeMasterId);
      for (const srcId of sourcesToMerge) {
        await mergePeopleDirectly({
          source_id: srcId,
          target_id: mergeMasterId,
          custom_name: mergeCustomName.trim() || undefined,
        });
      }
      showToast(currentLang === 'bg' ? '✓ Успешно обединяване!' : '✓ Merged successfully!');
      isMergeModalOpen = false;
      selectedPeopleIds = [];
      await loadData();
    } catch {
      showToast(currentLang === 'bg' ? 'Грешка при обединяване' : 'Merge error');
    } finally {
      isMerging = false;
    }
  }

  async function handleDeletePerson(p: Person) {
    if (!confirm(currentLang === 'bg' ? `Сигурни ли сте, че искате да премахнете ${p.canonical_name}?` : `Delete ${p.canonical_name}?`)) return;
    try {
      await deletePerson(p.id);
      people = people.filter((item) => item.id !== p.id);
      showToast(currentLang === 'bg' ? '✓ Премахнато' : '✓ Deleted');
    } catch {
      showToast(currentLang === 'bg' ? 'Грешка при премахване' : 'Delete error');
    }
  }

  async function handleSaveNewPerson() {
    if (!newName.trim()) return;
    const tagsArr = newTagsStr
      .split(',')
      .map((t) => t.trim().replace(/^\[|\]$/g, ''))
      .filter(Boolean);
    try {
      const res = await createPerson({
        canonical_name: newName.trim(),
        role: newRole.trim() || undefined,
        category: newCategory,
        rating: newRating,
        tags: tagsArr,
        notes: newNotes.trim() || undefined,
        public_figure: true,
        entity_type: newEntityType,
      });
      if (res && res.id) {
        showToast(currentLang === 'bg' ? '✓ Лицето е добавено!' : '✓ Person created!');
        isCreatingNew = false;
        newName = '';
        newRole = '';
        newTagsStr = '';
        newNotes = '';
        await loadData();
      }
    } catch {
      showToast(currentLang === 'bg' ? 'Грешка при създаване' : 'Creation error');
    }
  }

  async function handleSaveEditedPerson() {
    if (!editingPerson || !editingPerson.canonical_name.trim()) return;
    try {
      await updatePerson(editingPerson.id, editingPerson);
      showToast(currentLang === 'bg' ? '✓ Промените са запазени!' : '✓ Saved!');
      editingPerson = null;
      await loadData();
    } catch {
      showToast(currentLang === 'bg' ? 'Грешка при запазване' : 'Save error');
    }
  }
</script>

<div class="people-screen">
  <header class="screen-header">
    <div class="header-titles">
      <h2>👥 {currentLang === 'bg' ? 'Лица, институции и обекти' : 'People, Institutions & Objects'}</h2>
      <p class="header-desc">
        {currentLang === 'bg'
          ? 'Споменати личности, държавни органи, инфраструктура, сгради и теми във видеата.'
          : 'Mentioned figures, institutions, infrastructure, buildings, and topics across videos.'}
      </p>
    </div>

    <div class="header-actions">
      {#if toastMsg}
        <span class="screen-toast">{toastMsg}</span>
      {/if}
      <button class="btn btn-secondary btn-sm" onclick={handleAutoClassify} disabled={isClassifying}>
        {isClassifying ? '⏳...' : '🤖 ' + (currentLang === 'bg' ? 'Подреди по вид' : 'Auto-classify')}
      </button>
      <button class="btn btn-secondary btn-sm" onclick={handleAutoDedup} disabled={isAutoDeduping} title="Обединяване на дубликати и разделяне на съставни имена">
        {isAutoDeduping ? '⏳...' : '⚡ ' + (currentLang === 'bg' ? 'Обедини дубликати' : 'Deduplicate')}
      </button>
      <button class="btn btn-primary btn-sm" onclick={() => (isCreatingNew = true)}>
        + {currentLang === 'bg' ? 'Добави запис' : 'Add Entry'}
      </button>
    </div>
  </header>

  <div class="filter-bar">
    <div class="search-input-wrap">
      <input
        type="search"
        placeholder={currentLang === 'bg' ? 'Търсене по име, длъжност или таг...' : 'Search by name, role, tag...'}
        bind:value={searchQuery}
        onkeydown={(e) => e.key === 'Enter' && handleSearch()}
      />
      <button class="btn btn-secondary btn-sm" onclick={handleSearch}>
        🔍 {currentLang === 'bg' ? 'Търси' : 'Search'}
      </button>
    </div>

    <div class="select-group">
      <select bind:value={selectedEntityType} onchange={loadData}>
        {#each ENTITY_TYPES as et}
          <option value={et.id}>{et.icon} {currentLang === 'bg' ? et.bg : et.en}</option>
        {/each}
      </select>

      <select bind:value={selectedCategory} onchange={loadData}>
        {#each CATEGORIES as cat}
          <option value={cat.id}>{currentLang === 'bg' ? cat.bg : cat.en}</option>
        {/each}
      </select>

      <select bind:value={selectedSort} onchange={loadData}>
        <option value="rating">{currentLang === 'bg' ? 'Сортирай по Рейтинг' : 'Sort by Rating'}</option>
        <option value="mentions">{currentLang === 'bg' ? 'Сортирай по Брой споменавания' : 'Sort by Mentions'}</option>
        <option value="researched">{currentLang === 'bg' ? 'Последно проучени' : 'Recently Researched'}</option>
        <option value="name">{currentLang === 'bg' ? 'Азбучен ред (Име)' : 'Alphabetical'}</option>
      </select>
    </div>
  </div>

  <main class="people-grid-container">
    {#if isLoading}
      <div class="loading-state">
        <div class="spinner"></div>
        <span>{currentLang === 'bg' ? 'Зареждане на списъка...' : 'Loading people...'}</span>
      </div>
    {:else if people.length === 0}
      <div class="empty-state">
        <p>{currentLang === 'bg' ? 'Няма намерени субекти според избраните критерии.' : 'No entities found.'}</p>
        <button class="btn btn-secondary btn-sm" onclick={() => (isCreatingNew = true)}>
          + {currentLang === 'bg' ? 'Създай първи запис' : 'Create first entity'}
        </button>
      </div>
    {:else}
      <div class="people-grid">
        {#each people as person (person.id)}
          {@const entInfo = getEntityTypeInfo(person.entity_type)}
          <div class="person-card" class:card-selected={selectedPeopleIds.includes(person.id)}>
            <div class="card-head">
              <div class="person-identity">
                <input
                  type="checkbox"
                  class="person-select-checkbox"
                  checked={selectedPeopleIds.includes(person.id)}
                  onchange={() => togglePersonSelection(person.id)}
                  title="Избери за обединяване"
                />
                <span class="person-avatar">{person.canonical_name.slice(0, 2).toUpperCase()}</span>
                <div>
                  <h3 class="person-name">{person.canonical_name}</h3>
                  <div class="person-badges-row">
                    <span class="person-entity-badge {entInfo.cssClass}">{entInfo.icon} {entInfo.label}</span>
                    {#if person.category}
                      <span class="person-category-badge">{person.category}</span>
                    {/if}
                  </div>
                </div>
              </div>

              <div class="rating-widget" title="Приоритетен рейтинг на лицето (0-100)">
                <span class="rating-label">Рейтинг:</span>
                <span class="rating-value">{person.rating ?? 50}</span>
                <input
                  type="range"
                  min="0"
                  max="100"
                  value={person.rating ?? 50}
                  onchange={(e) => handleRatingChange(person, Number((e.target as HTMLInputElement).value))}
                  class="rating-slider"
                />
              </div>
            </div>

            {#if person.role}
              <div class="person-role">
                💼 {person.role}
              </div>
            {/if}

            <div class="tags-section">
              {#if person.tags && person.tags.length > 0}
                {#each person.tags as tag}
                  <span class="tag-pill">
                    #{tag}
                    <button class="tag-remove-btn" onclick={() => handleRemoveTag(person, tag)}>✕</button>
                  </span>
                {/each}
              {/if}
              <button class="tag-add-btn" onclick={() => handleQuickAddTag(person)}>
                + {currentLang === 'bg' ? 'таг' : 'tag'}
              </button>
            </div>

            {#if person.summary}
              <p class="person-summary">
                {person.summary}
              </p>
            {/if}

            <div class="claims-stat-bar">
              <span class="stat-item" title="Брой видеа със споменаване">
                🎬 {person.mentions ?? 0} {currentLang === 'bg' ? 'видеа' : 'videos'}
              </span>
              {#if person.researched_at}
                <span class="stat-item stat-researched" title={person.researched_at}>
                  ✓ {currentLang === 'bg' ? 'Проучен' : 'Researched'}
                </span>
              {/if}
            </div>

            <div class="card-actions">
              <button
                class="btn btn-primary btn-sm"
                onclick={() => handleTriggerResearch(person)}
                disabled={isJobActive('person-research', person.id)}
                title="Изготви аналитична справка от публични регистри и източници"
              >
                {isJobActive('person-research', person.id)
                  ? '⏳ ' + (currentLang === 'bg' ? 'В ход...' : 'Running...')
                  : '🔄 ' + (currentLang === 'bg' ? 'Проучи' : 'Research')}
              </button>

              <button
                class="btn btn-secondary btn-sm"
                onclick={() => (viewingDossier = person)}
                title="Отвори пълната аналитична справка"
              >
                📖 {currentLang === 'bg' ? 'Справка' : 'Report'}
              </button>

              <button
                class="btn btn-secondary btn-sm"
                onclick={() => (editingPerson = { ...person })}
                title="Редактирай данни"
              >
                ✏️
              </button>

              <button
                class="btn btn-danger btn-sm"
                onclick={() => handleDeletePerson(person)}
                title="Премахни лицето"
              >
                ✕
              </button>
            </div>
          </div>
        {/each}
      </div>
    {/if}

    {#if selectedPeopleIds.length > 0}
      <div class="floating-bulk-bar">
        <span class="bulk-count">
          <strong>{selectedPeopleIds.length}</strong> {currentLang === 'bg' ? 'избрани записа' : 'selected'}
        </span>
        {#if selectedPeopleIds.length >= 2}
          <button class="btn btn-primary btn-sm" onclick={openMergeModal}>
            🔗 {currentLang === 'bg' ? 'Обедини избраните' : 'Merge selected'}
          </button>
        {/if}
        <button class="btn btn-outline btn-sm" onclick={clearSelection}>
          ✕ {currentLang === 'bg' ? 'Отказ' : 'Clear'}
        </button>
      </div>
    {/if}
  </main>
</div>

{#if isMergeModalOpen}
  <div class="modal-backdrop" onclick={() => (isMergeModalOpen = false)} role="presentation">
    <div class="modal-dialog" onclick={(e) => e.stopPropagation()} onkeydown={(e) => e.stopPropagation()} role="dialog" tabindex="-1">
      <div class="modal-head">
        <h3>🔗 {currentLang === 'bg' ? 'Обединяване на избраните лица' : 'Merge Selected Entities'}</h3>
        <button class="close-btn" onclick={() => (isMergeModalOpen = false)}>✕</button>
      </div>

      <div class="modal-body">
        <p class="merge-modal-subhint">
          {currentLang === 'bg'
            ? 'Изберете водещ запис. Всички видеа, тагове и цитати от останалите ще бъдат прехвърлени към него, а дублиращите се записи ще бъдат изчистени.'
            : 'Select the primary record to keep. Videos, tags, and citations will be merged into it.'}
        </p>

        <div class="merge-candidates-list">
          {#each selectedPeopleIds as id}
            {@const p = people.find(item => item.id === id)}
            {#if p}
              <label class="merge-radio-row" class:is-master={mergeMasterId === p.id}>
                <input
                  type="radio"
                  name="master_selection"
                  value={p.id}
                  checked={mergeMasterId === p.id}
                  onchange={() => {
                    mergeMasterId = p.id;
                    mergeCustomName = p.canonical_name;
                  }}
                />
                <div class="radio-info">
                  <strong>{p.canonical_name}</strong>
                  <span class="radio-sub">ID: {p.id} • {p.mentions ?? 0} видеа • {p.category || 'Без категория'}</span>
                </div>
                {#if mergeMasterId === p.id}
                  <span class="master-tag">★ {currentLang === 'bg' ? 'Водещ запис' : 'Primary'}</span>
                {/if}
              </label>
            {/if}
          {/each}
        </div>

        <label class="merge-name-field">
          <span>{currentLang === 'bg' ? 'Окончателно канонично име:' : 'Final canonical name:'}</span>
          <input type="text" bind:value={mergeCustomName} class="merge-name-input" />
        </label>
      </div>

      <div class="modal-foot">
        <button class="btn btn-outline btn-sm" onclick={() => (isMergeModalOpen = false)} disabled={isMerging}>
          ✕ {currentLang === 'bg' ? 'Отказ' : 'Cancel'}
        </button>
        <button class="btn btn-primary btn-sm" onclick={handleExecuteMerge} disabled={isMerging}>
          {isMerging ? '⏳...' : '✓ ' + (currentLang === 'bg' ? 'Потвърди обединяването' : 'Confirm Merge')}
        </button>
      </div>
    </div>
  </div>
{/if}

{#if isCreatingNew}
  <div class="modal-backdrop" onclick={() => (isCreatingNew = false)} role="presentation">
    <div class="modal-dialog" onclick={(e) => e.stopPropagation()} onkeydown={(e) => e.stopPropagation()} role="dialog" tabindex="-1">
      <div class="modal-head">
        <h3>+ {currentLang === 'bg' ? 'Добавяне на обект' : 'Add Subject'}</h3>
        <button class="close-btn" onclick={() => (isCreatingNew = false)}>✕</button>
      </div>

      <div class="modal-body">
        <label>
          <span>{currentLang === 'bg' ? 'Вид на обекта' : 'Subject Type'}</span>
          <select bind:value={newEntityType}>
            {#each ENTITY_TYPES.filter((e) => e.id !== 'all') as e}
              <option value={e.id}>{e.icon} {currentLang === 'bg' ? e.bg : e.en}</option>
            {/each}
          </select>
        </label>

        <label>
          <span>{currentLang === 'bg' ? 'Наименование / Име' : 'Canonical Name'} *</span>
          <input type="text" bind:value={newName} placeholder="напр. Андрей Гюров, АМ Хемус, Национална детска болница" />
        </label>

        <label>
          <span>{currentLang === 'bg' ? 'Длъжност / Описание' : 'Role / Description'}</span>
          <input type="text" bind:value={newRole} placeholder="напр. Подуправител на БНБ, автомагистрала, обществена сграда" />
        </label>

        <label>
          <span>{currentLang === 'bg' ? 'Категория' : 'Category'}</span>
          <select bind:value={newCategory}>
            {#each CATEGORIES.filter((c) => c.id !== 'all') as c}
              <option value={c.id}>{currentLang === 'bg' ? c.bg : c.en}</option>
            {/each}
          </select>
        </label>

        <label>
          <span>{currentLang === 'bg' ? 'Рейтинг на важност (0 - 100)' : 'Priority Rating'}</span>
          <input type="range" min="0" max="100" bind:value={newRating} />
          <span class="range-hint">{newRating}</span>
        </label>

        <label>
          <span>{currentLang === 'bg' ? 'Тагове (разделени със запетая)' : 'Tags (comma separated)'}</span>
          <input type="text" bind:value={newTagsStr} placeholder="[десен], [парламент], [бивш министър]" />
        </label>

        <label>
          <span>{currentLang === 'bg' ? 'Бележки на анализатора' : 'Notes'}</span>
          <textarea bind:value={newNotes} rows="3"></textarea>
        </label>
      </div>

      <div class="modal-foot">
        <button class="btn btn-secondary btn-sm" onclick={() => (isCreatingNew = false)}>
          {currentLang === 'bg' ? 'Отказ' : 'Cancel'}
        </button>
        <button class="btn btn-primary btn-sm" onclick={handleSaveNewPerson} disabled={!newName.trim()}>
          {currentLang === 'bg' ? 'Създай' : 'Create'}
        </button>
      </div>
    </div>
  </div>
{/if}

{#if editingPerson}
  <div class="modal-backdrop" onclick={() => (editingPerson = null)} role="presentation">
    <div class="modal-dialog" onclick={(e) => e.stopPropagation()} onkeydown={(e) => e.stopPropagation()} role="dialog" tabindex="-1">
      <div class="modal-head">
        <h3>✏️ {currentLang === 'bg' ? 'Редактиране на' : 'Edit'} {editingPerson.canonical_name}</h3>
        <button class="close-btn" onclick={() => (editingPerson = null)}>✕</button>
      </div>

      <div class="modal-body">
        <label>
          <span>{currentLang === 'bg' ? 'Тип на субекта' : 'Entity Type'}</span>
          <select bind:value={editingPerson.entity_type}>
            {#each ENTITY_TYPES.filter((e) => e.id !== 'all') as e}
              <option value={e.id}>{e.icon} {currentLang === 'bg' ? e.bg : e.en}</option>
            {/each}
          </select>
        </label>

        <label>
          <span>{currentLang === 'bg' ? 'Име' : 'Name'}</span>
          <input type="text" bind:value={editingPerson.canonical_name} />
        </label>

        <label>
          <span>{currentLang === 'bg' ? 'Длъжност' : 'Role'}</span>
          <input type="text" bind:value={editingPerson.role} />
        </label>

        <label>
          <span>{currentLang === 'bg' ? 'Категория' : 'Category'}</span>
          <select bind:value={editingPerson.category}>
            {#each CATEGORIES.filter((c) => c.id !== 'all') as c}
              <option value={c.id}>{currentLang === 'bg' ? c.bg : c.en}</option>
            {/each}
          </select>
        </label>

        <label>
          <span>{currentLang === 'bg' ? 'Рейтинг' : 'Rating'} ({editingPerson.rating ?? 50})</span>
          <input type="range" min="0" max="100" bind:value={editingPerson.rating} />
        </label>

        <label>
          <span>{currentLang === 'bg' ? 'Кратко резюме' : 'Summary'}</span>
          <textarea bind:value={editingPerson.summary} rows="3"></textarea>
        </label>

        <label>
          <span>{currentLang === 'bg' ? 'Бележки' : 'Notes'}</span>
          <textarea bind:value={editingPerson.notes} rows="2"></textarea>
        </label>
      </div>

      <div class="modal-foot">
        <button class="btn btn-secondary btn-sm" onclick={() => (editingPerson = null)}>
          {currentLang === 'bg' ? 'Отказ' : 'Cancel'}
        </button>
        <button class="btn btn-primary btn-sm" onclick={handleSaveEditedPerson}>
          {currentLang === 'bg' ? 'Запази промените' : 'Save Changes'}
        </button>
      </div>
    </div>
  </div>
{/if}

{#if viewingDossier}
  <PersonDossierModal
    person={viewingDossier}
    {currentLang}
    onClose={() => (viewingDossier = null)}
    onResearchTrigger={async (p) => {
      await handleTriggerResearch(p);
      const found = people.find((item) => item.id === p.id);
      if (found) viewingDossier = found;
    }}
  />
{/if}

<style>
  .people-screen {
    flex: 1;
    display: flex;
    flex-direction: column;
    height: 100%;
    overflow: hidden;
    background: var(--bg-canvas);
    color: var(--text-body);
  }

  .screen-header {
    display: flex;
    align-items: center;
    justify-content: space-between;
    padding: 1rem 1.5rem;
    background: var(--surface-bg);
    border-bottom: 1px solid var(--border-color);
  }

  .header-titles h2 {
    margin: 0;
    font-size: 1.15rem;
    font-weight: 700;
    color: var(--text-heading);
  }

  .header-desc {
    margin: 0.2rem 0 0;
    font-size: 0.8rem;
    color: var(--text-muted);
  }

  .header-actions {
    display: flex;
    align-items: center;
    gap: 0.75rem;
  }

  .screen-toast {
    font-size: 0.78rem;
    font-weight: 600;
    color: var(--primary-accent);
    background: var(--primary-accent-alpha);
    padding: 0.3rem 0.6rem;
    border-radius: 4px;
    border: 1px solid var(--primary-accent);
  }

  .filter-bar {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 1rem;
    padding: 0.75rem 1.5rem;
    background: var(--surface-bg);
    border-bottom: 1px solid var(--border-color);
  }

  .search-input-wrap {
    display: flex;
    gap: 0.4rem;
    flex: 1;
    max-width: 480px;
  }

  .search-input-wrap input {
    flex: 1;
    padding: 0.4rem 0.75rem;
    border-radius: 6px;
    border: 1px solid var(--border-color);
    background: var(--input-bg);
    color: var(--text-body);
    font-size: 0.85rem;
  }

  .select-group {
    display: flex;
    gap: 0.5rem;
  }

  .select-group select {
    padding: 0.4rem 0.75rem;
    border-radius: 6px;
    border: 1px solid var(--border-color);
    background: var(--input-bg);
    color: var(--text-body);
    font-size: 0.82rem;
  }

  .people-grid-container {
    flex: 1;
    overflow-y: auto;
    padding: 1.25rem 1.5rem;
  }

  .people-grid {
    display: grid;
    grid-template-columns: repeat(auto-fill, minmax(var(--people-card-min, 350px), 1fr));
    gap: 1rem;
  }

  :global([data-display-profile='13']) .people-grid,
  :global([data-display-profile='14']) .people-grid {
    gap: 0.75rem;
  }

  :global([data-display-profile='18']) .people-grid {
    gap: 0.85rem;
  }

  :global([data-display-profile='13']) .person-card,
  :global([data-display-profile='14']) .person-card,
  :global([data-display-profile='18']) .person-card {
    padding: 0.85rem;
  }

  .person-card {
    background: var(--card-bg);
    border: 1px solid var(--border-color);
    border-radius: 8px;
    padding: 1rem;
    display: flex;
    flex-direction: column;
    gap: 0.6rem;
    transition: border-color 0.15s ease;
  }

  .person-card:hover {
    border-color: var(--primary-accent);
  }

  .card-head {
    display: flex;
    align-items: flex-start;
    justify-content: space-between;
    gap: 0.5rem;
  }

  .person-identity {
    display: flex;
    align-items: center;
    gap: 0.65rem;
  }

  .person-avatar {
    width: 38px;
    height: 38px;
    border-radius: 50%;
    background: var(--primary-accent);
    color: #fff;
    display: flex;
    align-items: center;
    justify-content: center;
    font-weight: 700;
    font-size: 0.85rem;
    flex-shrink: 0;
  }

  .person-name {
    margin: 0;
    font-size: 0.98rem;
    font-weight: 700;
    color: var(--text-heading);
  }

  .person-badges-row {
    display: flex;
    align-items: center;
    gap: 0.35rem;
    margin-top: 2px;
    flex-wrap: wrap;
  }

  .person-entity-badge {
    font-size: 0.68rem;
    font-weight: 700;
    padding: 1px 6px;
    border-radius: 4px;
    display: inline-flex;
    align-items: center;
    gap: 0.2rem;
  }

  .person-entity-badge.type-person {
    background: rgba(37, 99, 235, 0.12);
    color: var(--primary-accent);
    border: 1px solid rgba(37, 99, 235, 0.3);
  }

  .person-entity-badge.type-institution {
    background: rgba(147, 51, 234, 0.12);
    color: #9333ea;
    border: 1px solid rgba(147, 51, 234, 0.3);
  }

  .person-entity-badge.type-channel {
    background: rgba(234, 88, 12, 0.12);
    color: #ea580c;
    border: 1px solid rgba(234, 88, 12, 0.3);
  }

  .person-entity-badge.type-country {
    background: rgba(16, 185, 129, 0.12);
    color: #10b981;
    border: 1px solid rgba(16, 185, 129, 0.3);
  }

  .person-entity-badge.type-group {
    background: rgba(217, 119, 6, 0.12);
    color: #d97706;
    border: 1px solid rgba(217, 119, 6, 0.3);
  }

  .person-entity-badge.type-event {
    background: rgba(236, 72, 153, 0.12);
    color: #ec4899;
    border: 1px solid rgba(236, 72, 153, 0.3);
  }

  .person-entity-badge.type-infrastructure {
    background: rgba(14, 165, 233, 0.14);
    color: #0ea5e9;
    border: 1px solid rgba(14, 165, 233, 0.32);
  }

  .person-entity-badge.type-building {
    background: rgba(20, 184, 166, 0.14);
    color: #14b8a6;
    border: 1px solid rgba(20, 184, 166, 0.32);
  }

  .person-category-badge {
    font-size: 0.7rem;
    font-weight: 600;
    color: var(--text-muted);
    background: var(--pill-bg);
    border: 1px solid var(--border-color);
    padding: 1px 6px;
    border-radius: 4px;
  }

  .rating-widget {
    display: flex;
    flex-direction: column;
    align-items: flex-end;
    gap: 2px;
  }

  .rating-label {
    font-size: 0.68rem;
    color: var(--text-muted);
  }

  .rating-value {
    font-size: 0.82rem;
    font-weight: 700;
    color: var(--primary-accent);
  }

  .rating-slider {
    width: 80px;
    height: 4px;
    cursor: pointer;
  }

  .person-role {
    font-size: 0.82rem;
    color: var(--text-body);
  }

  .tags-section {
    display: flex;
    flex-wrap: wrap;
    gap: 0.35rem;
  }

  .tag-pill {
    display: inline-flex;
    align-items: center;
    gap: 4px;
    font-size: 0.7rem;
    font-weight: 600;
    color: var(--primary-accent);
    background: var(--primary-accent-alpha);
    border: 1px solid var(--primary-accent);
    padding: 1px 6px;
    border-radius: 4px;
  }

  .tag-remove-btn {
    background: transparent;
    border: none;
    color: var(--text-muted);
    font-size: 0.7rem;
    cursor: pointer;
    padding: 0;
  }

  .tag-remove-btn:hover {
    color: var(--verdict-contradicted);
  }

  .tag-add-btn {
    background: var(--pill-bg);
    border: 1px dashed var(--border-color);
    color: var(--text-muted);
    font-size: 0.7rem;
    padding: 1px 6px;
    border-radius: 4px;
    cursor: pointer;
  }

  .tag-add-btn:hover {
    color: var(--primary-accent);
    border-color: var(--primary-accent);
  }

  .person-summary {
    margin: 0;
    font-size: 0.8rem;
    color: var(--text-muted);
    line-height: 1.4;
    display: -webkit-box;
    -webkit-line-clamp: 3;
    line-clamp: 3;
    -webkit-box-orient: vertical;
    overflow: hidden;
  }

  .claims-stat-bar {
    display: flex;
    align-items: center;
    gap: 0.75rem;
    font-size: 0.74rem;
    color: var(--text-muted);
    border-top: 1px solid var(--border-color);
    padding-top: 0.4rem;
  }

  .stat-researched {
    color: var(--verdict-supported);
    font-weight: 600;
  }

  .card-actions {
    display: flex;
    gap: 0.4rem;
    margin-top: auto;
  }

  .modal-backdrop {
    position: fixed;
    inset: 0;
    background: rgba(0, 0, 0, 0.6);
    z-index: 100;
    display: flex;
    align-items: center;
    justify-content: center;
    padding: 1.5rem;
  }

  .modal-dialog {
    background: var(--surface-bg);
    border: 1px solid var(--border-color);
    border-radius: 8px;
    width: 100%;
    max-width: 520px;
    max-height: 90vh;
    display: flex;
    flex-direction: column;
    box-shadow: 0 10px 30px rgba(0, 0, 0, 0.4);
  }

  .modal-head {
    display: flex;
    align-items: center;
    justify-content: space-between;
    padding: 0.85rem 1.25rem;
    border-bottom: 1px solid var(--border-color);
  }

  .modal-head h3 {
    margin: 0;
    font-size: 1rem;
    color: var(--text-heading);
  }

  .modal-body {
    padding: 1.25rem;
    overflow-y: auto;
    display: flex;
    flex-direction: column;
    gap: 0.85rem;
  }

  .modal-body label {
    display: flex;
    flex-direction: column;
    gap: 0.3rem;
    font-size: 0.82rem;
    color: var(--text-heading);
    font-weight: 600;
  }

  .modal-body input,
  .modal-body select,
  .modal-body textarea {
    padding: 0.45rem 0.65rem;
    border-radius: 6px;
    border: 1px solid var(--border-color);
    background: var(--input-bg);
    color: var(--text-body);
    font-size: 0.85rem;
  }

  .modal-foot {
    display: flex;
    align-items: center;
    justify-content: flex-end;
    gap: 0.5rem;
    padding: 0.75rem 1.25rem;
    border-top: 1px solid var(--border-color);
  }

  .close-btn {
    background: transparent;
    border: none;
    font-size: 1.1rem;
    color: var(--text-muted);
    cursor: pointer;
  }

  .loading-state,
  .empty-state {
    display: flex;
    flex-direction: column;
    align-items: center;
    justify-content: center;
    height: 300px;
    gap: 0.75rem;
    color: var(--text-muted);
  }

  .spinner {
    width: 24px;
    height: 24px;
    border: 3px solid var(--border-color);
    border-top-color: var(--primary-accent);
    border-radius: 50%;
    animation: spin 0.7s linear infinite;
  }

  .person-select-checkbox {
    width: 1.15rem;
    height: 1.15rem;
    margin-right: 0.35rem;
    cursor: pointer;
    accent-color: var(--primary-accent, #3b82f6);
  }

  .person-card.card-selected {
    border-color: var(--primary-accent, #3b82f6);
    box-shadow: 0 0 10px rgba(59, 130, 246, 0.25);
  }

  .floating-bulk-bar {
    position: fixed;
    bottom: 1.5rem;
    left: 50%;
    transform: translateX(-50%);
    background: #1e293b;
    border: 1px solid #3b82f6;
    box-shadow: 0 8px 24px rgba(0, 0, 0, 0.45);
    border-radius: 9999px;
    padding: 0.5rem 1.25rem;
    display: flex;
    align-items: center;
    gap: 0.9rem;
    z-index: 1000;
  }

  .bulk-count {
    font-size: 0.85rem;
    color: #e2e8f0;
  }

  .merge-modal-subhint {
    font-size: 0.82rem;
    color: var(--text-secondary, #94a3b8);
    margin-bottom: 0.8rem;
    line-height: 1.4;
  }

  .merge-candidates-list {
    display: flex;
    flex-direction: column;
    gap: 0.45rem;
    margin-bottom: 0.9rem;
  }

  .merge-radio-row {
    display: flex;
    align-items: center;
    gap: 0.6rem;
    padding: 0.5rem 0.75rem;
    background: rgba(255, 255, 255, 0.03);
    border: 1px solid var(--border-color);
    border-radius: 6px;
    cursor: pointer;
    transition: background 0.15s ease;
  }

  .merge-radio-row:hover {
    background: rgba(255, 255, 255, 0.06);
  }

  .merge-radio-row.is-master {
    border-color: #3b82f6;
    background: rgba(59, 130, 246, 0.1);
  }

  .radio-info {
    display: flex;
    flex-direction: column;
    gap: 2px;
    flex: 1;
  }

  .radio-sub {
    font-size: 0.72rem;
    color: var(--text-secondary, #94a3b8);
  }

  .master-tag {
    font-size: 0.72rem;
    font-weight: 700;
    color: #60a5fa;
    background: rgba(59, 130, 246, 0.2);
    padding: 2px 6px;
    border-radius: 4px;
  }

  .merge-name-field {
    display: flex;
    flex-direction: column;
    gap: 0.35rem;
    font-size: 0.82rem;
  }

  .merge-name-input {
    background: rgba(0, 0, 0, 0.2);
    border: 1px solid var(--border-color);
    border-radius: 6px;
    padding: 0.45rem 0.65rem;
    color: inherit;
    font-size: 0.88rem;
  }

  @keyframes spin {
    to {
      transform: rotate(360deg);
    }
  }
</style>