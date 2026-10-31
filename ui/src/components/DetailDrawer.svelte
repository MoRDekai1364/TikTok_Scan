<script lang="ts">
  import {
    aiCheckTranscript,
    assessCreatorRisk,
    deleteEvidence,
    deleteVideo,
    fetchDeepfakeResult,
    fetchEvidencePreview,
    fetchPeople,
    fetchVideoDetail,
    fetchVideosByCreator,
    getCoreUrl,
    googleAiFixTranscript,
    googleAudioTranscribe,
    logDecision,
    openExternalUrl,
    queryGoogleAi,
    reassignVideoPerson,
    redactClaim,
    removePersonFromVideo,
    researchPerson,
    retranscribeVideo,
    setCreatorWatchlist,
    submitClaimFeedback,
    triggerAegisScan,
    updateTranscript,
  } from '../lib/api';
  import { isJobActive, startAiJob, finishAiJob, getActiveAiJobs } from '../lib/aiTasks.svelte';
  import {
    registerPlaybackElement,
    unregisterPlaybackElement,
    updatePlaybackProgress,
    setPlaybackPlaying,
  } from '../lib/playback.svelte';
  import type { GoogleAiQueryResult } from '../lib/api';
  import type { Language } from '../lib/i18n';
  import { translations } from '../lib/i18n';
  import type { AegisDeepfakeResult, ClaimWithEvidence, EntityType, LeadCard, OcrText, Person, VideoDetail } from '../lib/types';
  import { formatVerdictDisplay } from '../lib/types';
  import { highlightPlainTextToHtml } from '../lib/reportHighlighter';
  import { widescreen } from '../lib/widescreen.svelte';
  import BookReaderToc from './BookReaderToc.svelte';
  import type { TocItem } from './BookReaderToc.svelte';
  import PersonDossierModal from './PersonDossierModal.svelte';

  let {
    videoId,
    currentLang,
    onClose,
    onSelectVideo,
    onDiscard,
    onDeleteVideo,
  }: {
    videoId: string;
    currentLang: Language;
    onClose: () => void;
    onSelectVideo?: (id: string) => void;
    onDiscard?: (id: string) => void;
    onDeleteVideo?: (id: string) => void;
  } = $props();

  const t = $derived(translations[currentLang]);

  let detail = $state<VideoDetail | null>(null);
  let isLoading = $state(true);
  let isEditingTranscript = $state(false);
  let editedText = $state('');
  let isSavingTranscript = $state(false);
  let isAiChecking = $state(false);
  let isGoogleAiFixing = $state(false);
  let isGoogleAudioTranscribing = $state(false);
  let isRetranscribing = $state(false);
  let copyToast = $state<string | null>(null);
  let copyAllToast = $state<string | null>(null);
  let videoElement = $state<HTMLVideoElement | null>(null);
  let currentTimeMs = $state(0);
  let metadataOpen = $state(false);
  let activeClaimId = $state<number | null>(null);
  let highlightedSegmentMs = $state<number | null>(null);
  let creatorVideos = $state<LeadCard[]>([]);
  let isLoadingCreatorVideos = $state(false);
  let isTogglingWatchlist = $state(false);
  let isAssessingRisk = $state(false);
  let previewModalData = $state<{ url: string; title: string; text: string; domain: string } | null>(null);
  let googleAiModalData = $state<GoogleAiQueryResult | null>(null);
  let isGoogleAiQuerying = $state(false);
  let isLoadingPreview = $state(false);
  let isResearchingPersonId = $state<number | null>(null);
  let expandedEvidenceId = $state<number | null>(null);
  let activeEvidenceId = $state<number | null>(null);
  let feedbackSubmitted = $state<Record<number, string>>({});
  let feedbackPickerClaimId = $state<number | null>(null);
  let isPlaying = $state(false);
  let videoContainerRef = $state<HTMLElement | null>(null);
  let isVideoOutOfView = $state(false);
  let disambiguatingPerson = $state<Person | null>(null);
  let viewingPersonDossier = $state<Person | null>(null);
  let availablePeople = $state<Person[]>([]);
  let disambiguateNewName = $state('');
  let disambiguateRole = $state('');
  let disambiguateTags = $state('');
  let disambiguateTargetPersonId = $state<number | null>(null);
  let disambiguateEntityType = $state<EntityType>('person');
  let isAegisScanning = $state(false);
  let aegisScanError = $state<string | null>(null);
  let drawerBodyEl = $state<HTMLElement | null>(null);
  let previewBodyEl = $state<HTMLElement | null>(null);

  function getEntityTypeInfo(type?: string) {
    switch (type) {
      case 'institution':
        return { label: currentLang === 'bg' ? 'Институция' : 'Institution', icon: '🏛️', cssClass: 'type-institution' };
      case 'channel':
        return { label: currentLang === 'bg' ? 'Канал / Медия' : 'Channel / Media', icon: '📺', cssClass: 'type-channel' };
      case 'country':
        return { label: currentLang === 'bg' ? 'Държава' : 'Country', icon: '🌍', cssClass: 'type-country' };
      case 'group':
        return { label: currentLang === 'bg' ? 'Група / Партия' : 'Group / Party', icon: '👥', cssClass: 'type-group' };
      case 'event':
        return { label: currentLang === 'bg' ? 'Събитие' : 'Event', icon: '📅', cssClass: 'type-event' };
      case 'infrastructure':
        return { label: currentLang === 'bg' ? 'Инфраструктура' : 'Infrastructure', icon: '🛣️', cssClass: 'type-infrastructure' };
      case 'building':
        return { label: currentLang === 'bg' ? 'Сграда / Обект' : 'Building / Facility', icon: '🏢', cssClass: 'type-building' };
      default:
        return { label: currentLang === 'bg' ? 'Личност' : 'Person', icon: '👤', cssClass: 'type-person' };
    }
  }

  const transcriptDisplaySegments = $derived.by(() => {
    if (!detail?.transcript) return [];
    if (detail.transcript.segments && detail.transcript.segments.length > 0) {
      return detail.transcript.segments;
    }
    const full = detail.transcript.edited_text || detail.transcript.text || '';
    if (!full.trim()) return [];
    const clean = full.replace(/\*\*\d{2}:\d{2}\*\*\n*/g, '').trim();
    const sentences = clean.split(/(?<=[.!?])\s+/).filter(Boolean);
    const estDuration = (detail.video.duration_s || 60) * 1000;
    const step = Math.max(1000, Math.round(estDuration / Math.max(1, sentences.length)));
    return sentences.map((st, i) => ({
      start_ms: i * step,
      end_ms: (i + 1) * step,
      text: st,
    }));
  });

  function isDefiniteMeaningfulText(raw: string): boolean {
    const text = raw.trim();
    if (!text) return false;
    const words = text.split(/\s+/).filter(Boolean);
    if (words.length === 0) return false;

    const alphaChars = text.replace(/[^a-zA-Zа-яА-Я0-9]/g, '');
    if (alphaChars.length < 4) return false;

    const symbolCount = text.replace(/[a-zA-Zа-яА-Я0-9\s]/g, '').length;
    if (symbolCount / text.length > 0.28) return false;

    let singleCharCount = 0;
    let validWordsCount = 0;
    let cyrillicChars = 0;
    let latinChars = 0;

    for (const w of words) {
      const clean = w.replace(/[^a-zA-Zа-яА-Я]/g, '');
      if (clean.length === 1) singleCharCount++;
      else if (clean.length >= 3) validWordsCount++;

      for (const ch of clean) {
        if (/[а-яА-Я]/.test(ch)) cyrillicChars++;
        else if (/[a-zA-Z]/.test(ch)) latinChars++;
      }
    }

    if (words.length >= 3 && singleCharCount / words.length >= 0.35) return false;
    if (cyrillicChars > 0 && latinChars > 0 && words.length < 6 && singleCharCount >= 2) return false;
    if (validWordsCount < 1 && alphaChars.length < 6) return false;

    return true;
  }

  let showOcr = $state(false);

  const ocrItems = $derived.by((): OcrText[] => {
    const rows = detail?.ocr_texts ?? [];
    return rows
      .map((o) => ({ ...o, text: (o.text || '').trim() }))
      .filter((o) => isDefiniteMeaningfulText(o.text))
      .sort((a, b) => a.frame_ms - b.frame_ms);
  });

  $effect(() => {
    showOcr = ocrItems.length > 0;
  });

  const ocrBySegment = $derived.by(() => {
    const map = new Map<number, OcrText[]>();
    const tail: OcrText[] = [];
    const segs = transcriptDisplaySegments;
    for (const o of ocrItems) {
      let target = -1;
      for (const seg of segs) {
        if (seg.end_ms > o.frame_ms) {
          target = seg.start_ms;
          break;
        }
      }
      if (target < 0) {
        tail.push(o);
      } else {
        const list = map.get(target) ?? [];
        list.push(o);
        map.set(target, list);
      }
    }
    return { map, tail };
  });

  const timelineDurationMs = $derived.by(() => {
    const segs = transcriptDisplaySegments;
    const lastSeg = segs.length > 0 ? segs[segs.length - 1].end_ms : 0;
    const lastOcr = ocrItems.length > 0 ? ocrItems[ocrItems.length - 1].frame_ms + 2000 : 0;
    return Math.max((detail?.video.duration_s ?? 0) * 1000, lastSeg, lastOcr, 1);
  });

  let activeLoadId = '';

  $effect(() => {
    const id = videoId;
    if (id) {
      activeLoadId = id;
      loadDetail(id);
    }
  });

  async function loadDetail(id: string) {
    isLoading = true;
    isEditingTranscript = false;
    currentTimeMs = 0;
    creatorVideos = [];
    try {
      const res = await fetchVideoDetail(id);
      if (activeLoadId === id) {
        detail = res;
        if (res?.transcript) {
          editedText = res.transcript.edited_text || res.transcript.text;
        } else {
          editedText = '';
        }
        if (res?.creator?.id) {
          loadCreatorVideos(res.creator.id);
        } else if (res?.video.creator_handle) {
          loadCreatorVideos(res.video.creator_handle);
        }
        if (!res?.aegis_deepfake && id) {
          fetchDeepfakeResult(id).then((df) => {
            if (df && activeLoadId === id && detail) {
              detail = { ...detail, aegis_deepfake: df };
            } else if (!df && activeLoadId === id && detail && !isAegisScanning) {
              handleTriggerAegis();
            }
          }).catch(() => {
            if (activeLoadId === id && detail && !isAegisScanning) {
              handleTriggerAegis();
            }
          });
        }
      }
    } catch {
      if (activeLoadId === id) {
        detail = null;
      }
    } finally {
      if (activeLoadId === id) {
        isLoading = false;
      }
    }
  }

  interface ReasoningStatementItem {
    status: 'supported' | 'contradicted' | 'unverifiable';
    statement?: string;
    detail: string;
    passageRefs: string[];
  }

  interface ReasoningBreakdown {
    items: ReasoningStatementItem[];
    summary: string;
    extraNotes: string[];
  }

  function verdictColorKey(verdict?: string | null): 'supported' | 'contradicted' | 'unverifiable' | 'neutral' {
    const v = (verdict || '').toUpperCase();
    if (v === 'SUPPORTED') return 'supported';
    if (v === 'CONTRADICTED') return 'contradicted';
    if (v === 'UNVERIFIABLE' || v === 'ALREADY_CHECKED') return 'unverifiable';
    return 'neutral';
  }

  function classifySentenceStatus(sentence: string, fallbackVerdict?: string | null): 'supported' | 'contradicted' | 'unverifiable' {
    const low = sentence.toLowerCase();
    const hasNegationOrMissing =
      low.includes('не споменава') ||
      low.includes('няма данни') ||
      low.includes('не съдържа') ||
      low.includes('не дават конкретни') ||
      low.includes('не дава конкретни') ||
      low.includes('не е релевантен') ||
      low.includes('не може да се потвърди') ||
      low.includes('няма достатъчно') ||
      low.includes('липсва') ||
      low.includes('липса на') ||
      low.includes('непроверим') ||
      low.includes('противоречиви данни');
    const hasContradiction =
      low.includes('опроверга') ||
      low.includes('невярно') ||
      low.includes('не отговаря на истината') ||
      low.includes('оспорено') ||
      low.includes('завишен') ||
      low.includes('манипулиран') ||
      low.includes('фалшиф') ||
      low.includes('различно от') ||
      low.includes('обратното') ||
      low.includes('не съответства');
    const hasSupport =
      low.includes('потвърждава') ||
      low.includes('подкрепя') ||
      low.includes('съвпада') ||
      low.includes('доказва, че') ||
      low.includes('действително');

    if (hasNegationOrMissing && !low.includes('пряко опровержение')) {
      return 'unverifiable';
    }
    if (hasContradiction && !hasSupport) {
      return 'contradicted';
    }
    if (hasSupport && !hasContradiction) {
      return 'supported';
    }
    if (hasContradiction) {
      return 'contradicted';
    }
    const fb = verdictColorKey(fallbackVerdict);
    return fb === 'supported' || fb === 'contradicted' ? fb : 'unverifiable';
  }

  function extractPassageRefs(text: string): string[] {
    const matches = text.match(/\bp\d{1,2}\b/gi) || [];
    const unique = new Set<string>();
    for (const m of matches) {
      unique.add(m.toLowerCase());
    }
    return Array.from(unique);
  }

  function splitProseIntoSentences(text: string): string[] {
    const placeholders: string[] = [];
    let masked = text.replace(/https?:\/\/\S+/gi, (m) => {
      const cleanUrl = m.replace(/[.,;)]+$/, '');
      const trail = m.slice(cleanUrl.length);
      const idx = placeholders.push(cleanUrl) - 1;
      return `__URL_${idx}__${trail}`;
    });
    masked = masked.replace(/\b([a-z0-9-]+)\.(bg|com|org|net|eu|info|media|tv)\b/gi, '$1__DOT__$2');
    masked = masked.replace(/\b(г|чл|ал|т|пр|напр|вкл|др|млн|млрд|хил)\./gi, '$1__ABBR__');

    const rawParts = masked
      .split(/(?<=[.!?])\s+(?=[А-ЯA-Z0-9„"\[(])/u)
      .map((s) =>
        s
          .replace(/__DOT__/g, '.')
          .replace(/__ABBR__/g, '.')
          .replace(/__URL_(\d+)__/g, (_, i) => placeholders[Number(i)] || '')
          .trim()
      )
      .filter(Boolean);

    return rawParts;
  }

  function parseReasoningBreakdown(rawText: string, claimVerdict?: string | null): ReasoningBreakdown {
    const cleaned = (rawText || '').trim();
    if (!cleaned) {
      return { items: [], summary: '', extraNotes: [] };
    }

    const extraNotes: string[] = [];
    let working = cleaned;

    const consensusIdx = working.search(/\[(Мулти-моделен консенсус|Разминаване в консенсуса)\]:/);
    if (consensusIdx !== -1) {
      extraNotes.push(working.slice(consensusIdx).trim());
      working = working.slice(0, consensusIdx).trim();
    }

    const lines = working.split('\n').map((l) => l.trim()).filter(Boolean);
    const hasStructuredTags = lines.some((l) =>
      /^\[(ПОТВЪРДЕНО|ОПРОВЕРГАНО|НЕПРОВЕРИМО|ОБОБЩЕНИЕ)\]/.test(l)
    );

    if (hasStructuredTags) {
      const items: ReasoningStatementItem[] = [];
      const summaryParts: string[] = [];
      for (const line of lines) {
        const tagMatch = line.match(/^\[(ПОТВЪРДЕНО|ОПРОВЕРГАНО|НЕПРОВЕРИМО|ОБОБЩЕНИЕ)\]\s*(.*)$/);
        if (tagMatch) {
          const tag = tagMatch[1];
          const body = tagMatch[2].trim();
          if (tag === 'ОБОБЩЕНИЕ') {
            if (body) summaryParts.push(body);
          } else {
            const status: 'supported' | 'contradicted' | 'unverifiable' =
              tag === 'ПОТВЪРДЕНО' ? 'supported' : tag === 'ОПРОВЕРГАНО' ? 'contradicted' : 'unverifiable';
            const sepIdx = body.indexOf('::');
            if (sepIdx !== -1) {
              const st = body.slice(0, sepIdx).trim();
              const det = body.slice(sepIdx + 2).trim();
              items.push({
                status,
                statement: st || undefined,
                detail: det || st,
                passageRefs: extractPassageRefs(`${st} ${det}`),
              });
            } else {
              items.push({
                status,
                detail: body,
                passageRefs: extractPassageRefs(body),
              });
            }
          }
        } else {
          summaryParts.push(line);
        }
      }
      return {
        items,
        summary: summaryParts.join(' '),
        extraNotes,
      };
    }

    let trailingParen = '';
    const parenMatch = working.match(/\((Липсва[^)]+|Спорът[^)]+|Демографските[^)]+)\)\s*$/);
    if (parenMatch && parenMatch.index !== undefined) {
      trailingParen = parenMatch[0].trim();
      working = working.slice(0, parenMatch.index).trim();
    }

    const sentences = splitProseIntoSentences(working);
    if (sentences.length <= 1) {
      const single = [working, trailingParen].filter(Boolean).join(' ');
      return {
        items: [],
        summary: single,
        extraNotes,
      };
    }

    const conclusionRe = /^(следователно|поради|в заключение|в резултат|затова|липсата на|ето защо|общата оценка)/i;
    let splitAt = sentences.findIndex((s, idx) => idx > 0 && conclusionRe.test(s));
    if (splitAt === -1) {
      splitAt = sentences.length - 1;
    }

    const itemSentences = sentences.slice(0, splitAt);
    const summarySentences = sentences.slice(splitAt);
    if (trailingParen) {
      summarySentences.push(trailingParen);
    }

    const items: ReasoningStatementItem[] = itemSentences.map((s) => ({
      status: classifySentenceStatus(s, claimVerdict),
      detail: s,
      passageRefs: extractPassageRefs(s),
    }));

    return {
      items,
      summary: summarySentences.join(' '),
      extraNotes,
    };
  }

  function normalizeAegisVerdict(verdict?: string | null, audioSynth?: boolean, videoSynth?: boolean): 'deepfake' | 'suspicious' | 'authentic' {
    const v = (verdict || '').toLowerCase();
    if (
      audioSynth ||
      videoSynth ||
      v === 'deepfake' ||
      v === 'confirmed_deepfake' ||
      v === 'ai_generated_video' ||
      v === 'ai_voice_clone' ||
      v === 'robotic_tts'
    ) {
      return 'deepfake';
    }
    if (v === 'suspicious' || v === 'suspected_synthetic') {
      return 'suspicious';
    }
    return 'authentic';
  }

  async function handleTriggerAegis() {
    if (!detail?.video.id || isAegisScanning) return;
    const vid = detail.video.id;
    isAegisScanning = true;
    aegisScanError = null;
    startAiJob('aegis-scan', vid, currentLang === 'bg' ? `Проверка на записа: ${vid}` : `Media check: ${vid}`);
    try {
      const res = await triggerAegisScan(vid);
      if (res && detail) {
        detail = {
          ...detail,
          aegis_deepfake: res,
        };
      }
      finishAiJob('aegis-scan', vid, currentLang === 'bg' ? `✓ Проверката на записа (${vid}) завърши` : `✓ Media check (${vid}) completed`, true);
      copyToast = currentLang === 'bg' ? '🛡️ Проверката на записа завърши!' : '🛡️ Media scan completed!';
      setTimeout(() => (copyToast = null), 2500);
    } catch (err: any) {
      aegisScanError = err?.message || 'Error running AEGIS scan';
      finishAiJob('aegis-scan', vid, currentLang === 'bg' ? 'Грешка при проверка на записа' : 'Media scan error', false);
    } finally {
      isAegisScanning = false;
    }
  }

  async function loadCreatorVideos(creatorIdOrHandle: string) {
    isLoadingCreatorVideos = true;
    try {
      const list = await fetchVideosByCreator(creatorIdOrHandle, 8);
      creatorVideos = list.filter((v) => v.id !== videoId);
    } catch {
      creatorVideos = [];
    } finally {
      isLoadingCreatorVideos = false;
    }
  }

  function selectOtherVideo(id: string) {
    if (onSelectVideo) {
      onSelectVideo(id);
    } else {
      activeLoadId = id;
      loadDetail(id);
    }
  }

  function handleTimeUpdate() {
    if (videoElement) {
      currentTimeMs = Math.round(videoElement.currentTime * 1000);
      updatePlaybackProgress(videoElement.currentTime, videoElement.duration);
    }
  }

  $effect(() => {
    return () => {
      if (videoElement) {
        unregisterPlaybackElement(videoElement);
      }
    };
  });

  function seekToMs(ms: number) {
    if (videoElement) {
      videoElement.currentTime = ms / 1000;
      videoElement.play();
    }
  }

  function jumpToClaim(claimId: number) {
    activeClaimId = claimId;
    const matchedClaim = detail?.claims.find(c => c.claim.id === claimId);
    const nonJunkEv = (matchedClaim?.evidence || []).filter(ev => !isJunkSource(ev));
    const primaryEv = nonJunkEv.find(e => e.stance === 'supports' || e.stance === 'contradicts') || nonJunkEv[0];
    if (primaryEv) {
      activeEvidenceId = primaryEv.id;
      const evEl = document.getElementById(`evidence-${primaryEv.id}`);
      if (evEl) {
        evEl.scrollIntoView({ behavior: 'smooth', block: 'center' });
      }
      setTimeout(() => {
        if (activeEvidenceId === primaryEv.id) {
          activeEvidenceId = null;
        }
      }, 3500);
    } else {
      const el = document.getElementById(`claim-${claimId}`);
      if (el) {
        el.scrollIntoView({ behavior: 'smooth', block: 'center' });
      }
    }
    setTimeout(() => {
      if (activeClaimId === claimId) {
        activeClaimId = null;
      }
    }, 3500);
  }

  function jumpToSegment(startMs?: number | null) {
    if (startMs === undefined || startMs === null) return;
    seekToMs(startMs);
    highlightedSegmentMs = startMs;
    const el = document.getElementById(`seg-${startMs}`);
    if (el) {
      el.scrollIntoView({ behavior: 'smooth', block: 'center' });
    }
    setTimeout(() => {
      if (highlightedSegmentMs === startMs) {
        highlightedSegmentMs = null;
      }
    }, 2500);
  }

  function jumpToEvidence(evidenceId: number) {
    activeEvidenceId = evidenceId;
    const el = document.getElementById(`evidence-${evidenceId}`);
    if (el) {
      el.scrollIntoView({ behavior: 'smooth', block: 'center' });
    }
    setTimeout(() => {
      if (activeEvidenceId === evidenceId) {
        activeEvidenceId = null;
      }
    }, 2500);
  }

  async function handleSendFeedback(claimId: number, rating: 'correct' | 'wrong_verdict' | 'unrelated_evidence', suggestedVerdict?: string) {
    if (!detail) return;
    feedbackSubmitted[claimId] = rating;
    feedbackPickerClaimId = null;
    try {
      await submitClaimFeedback(claimId, {
        video_id: detail.video.id,
        rating,
        suggested_verdict: suggestedVerdict,
      });
    } catch {
    }
  }

  let redactingClaimId = $state<number | null>(null);
  let redactText = $state('');
  let redactNotes = $state('');
  let isRedacting = $state(false);

  function startRedact(claim: { id: number; text_bg: string; reasoning_bg?: string | null }) {
    redactingClaimId = claim.id;
    redactText = claim.text_bg;
    redactNotes = '';
  }

  let stmtEdit = $state<{ claimId: number; key: string; original: string; text: string; note: string } | null>(null);

  function startStmtEdit(claimId: number, idx: number, original: string, detailText: string) {
    stmtEdit = { claimId, key: `${claimId}-${idx}`, original, text: original, note: '' };
  }

  async function submitStmtEdit(claimText: string) {
    if (!detail || !stmtEdit || !stmtEdit.text.trim()) return;
    isRedacting = true;
    const e = stmtEdit;
    const notes =
      `Изявление: "${e.original}" -> коригирано от потребителя: "${e.text.trim()}".` +
      (e.note.trim() ? ` Бележка: ${e.note.trim()}` : '');
    try {
      await redactClaim(e.claimId, detail.video.id, claimText, notes);
      stmtEdit = null;
      setTimeout(async () => {
        const fresh = await fetchVideoDetail(videoId);
        if (fresh) detail = fresh;
      }, 4000);
    } catch {
    } finally {
      isRedacting = false;
    }
  }

  async function submitRedact() {
    if (!detail || redactingClaimId === null || !redactText.trim()) return;
    isRedacting = true;
    try {
      await redactClaim(redactingClaimId, detail.video.id, redactText, redactNotes);
      redactingClaimId = null;
      setTimeout(async () => {
        const fresh = await fetchVideoDetail(videoId);
        if (fresh) detail = fresh;
      }, 4000);
    } catch {
    } finally {
      isRedacting = false;
    }
  }

  function escapeHtml(str: string): string {
    return str
      .replace(/&/g, '&amp;')
      .replace(/</g, '&lt;')
      .replace(/>/g, '&gt;')
      .replace(/"/g, '&quot;')
      .replace(/'/g, '&#039;');
  }

  function escapeRegex(str: string): string {
    return str.replace(/[.*+?^${}()|[\]\\]/g, '\\$&');
  }

  function renderHighlightedSnippet(text: string, claimText: string): string {
    if (!text) return '';
    const kws = getMatchingKeywords(claimText, text);
    let safe = escapeHtml(text);
    if (kws.length === 0) return safe;
    for (const kw of kws) {
      if (kw.length >= 2) {
        const reg = new RegExp(`(${escapeRegex(kw)})`, 'gi');
        safe = safe.replace(reg, '<mark class="matched-passage">$1</mark>');
      }
    }
    return safe;
  }

  function stemSet(text: string): Set<string> {
    const out = new Set<string>();
    for (const w of text.toLowerCase().match(/[\p{L}\p{N}]{4,}/gu) ?? []) {
      out.add(w.slice(0, 5));
    }
    return out;
  }

  function claimSource(claimText: string): 'speech' | 'screen' | 'both' {
    const claimStems = [...stemSet(claimText)];
    if (claimStems.length === 0 || ocrItems.length === 0) return 'speech';
    const spoken = stemSet(detail?.transcript?.edited_text || detail?.transcript?.text || '');
    const shown = stemSet(ocrItems.map((o) => o.text).join(' '));
    const coverage = (pool: Set<string>) => claimStems.filter((s) => pool.has(s)).length / claimStems.length;
    const speechCov = coverage(spoken);
    const screenCov = coverage(shown);
    if (screenCov >= 0.5 && speechCov >= 0.5) return 'both';
    if (screenCov >= 0.4 && screenCov > speechCov + 0.15) return 'screen';
    if (speechCov === 0 && screenCov > 0) return 'screen';
    return 'speech';
  }

  function claimSourceLabel(src: 'speech' | 'screen' | 'both'): string {
    if (src === 'screen') return currentLang === 'bg' ? 'Текст на екрана' : 'On-screen text';
    if (src === 'both') return currentLang === 'bg' ? 'Реч + екран' : 'Speech + screen';
    return currentLang === 'bg' ? 'Реч' : 'Speech';
  }

  function getSegmentClaimInfo(seg: { start_ms: number; end_ms: number; text?: string }) {
    if (!detail?.claims || detail.claims.length === 0) return null;
    for (let i = 0; i < detail.claims.length; i++) {
      const c = detail.claims[i].claim;
      const s = c.start_ms ?? 0;
      const e = c.end_ms ?? 0;
      const timeMatch = (s > 0 || e > 0) && (
        (seg.start_ms >= s && seg.start_ms < e) ||
        (seg.end_ms > s && seg.end_ms <= e) ||
        (s >= seg.start_ms && e <= seg.end_ms)
      );
      const segText = (seg.text || '').trim().toLowerCase();
      const claimText = (c.text_bg || '').trim().toLowerCase();
      const textMatch = segText.length > 15 && (claimText.includes(segText) || segText.includes(claimText.slice(0, 30)));
      if (timeMatch || textMatch) {
        return {
          claimId: c.id,
          index: i + 1,
          verdict: (c.verdict || '').toLowerCase(),
          text: c.text_bg,
          source: claimSource(c.text_bg),
        };
      }
    }
    return null;
  }

  async function handleLoadEvidencePreview(url: string) {
    if (!url) return;
    isLoadingPreview = true;
    try {
      const res = await fetchEvidencePreview(url);
      previewModalData = res;
    } catch {
      copyToast = currentLang === 'bg' ? 'Грешка при зареждане на източника' : 'Failed to load source';
      setTimeout(() => (copyToast = null), 3000);
    } finally {
      isLoadingPreview = false;
    }
  }

  async function handleDeleteEvidence(claimId: number, evidenceId: number) {
    try {
      await deleteEvidence(evidenceId);
      if (detail?.claims) {
        const c = detail.claims.find(x => x.claim.id === claimId);
        if (c) {
          c.evidence = c.evidence.filter(e => e.id !== evidenceId);
        }
      }
      copyToast = currentLang === 'bg' ? '✓ Източникът беше премахнат' : '✓ Source removed';
      setTimeout(() => (copyToast = null), 2500);
    } catch {
      copyToast = currentLang === 'bg' ? 'Грешка при премахване на източника' : 'Failed to remove source';
      setTimeout(() => (copyToast = null), 2500);
    }
  }

  function isJunkSource(ev: any): boolean {
    const u = (ev.source_url || ev.url || '').toLowerCase();
    if (!u) return false;
    return u.includes('familylink') || u.includes('families.google') || u.includes('bing.com') ||
           u.includes('pornhub') || u.includes('xxx') || u.includes('casino') || u.includes('betano') ||
           u.includes('alo.bg') || u.includes('kompanionki') || u.includes('otvet.mail.ru') ||
           u.includes('badoo.com') || u.includes('playhop.com') || u.includes('bolshoyvopros');
  }

  function getMatchingKeywords(claimText: string, passageText: string): string[] {
    const stopWords = new Set([
      'това', 'след', 'като', 'беше', 'който', 'която', 'което', 'които', 'през', 'защото',
      'всичко', 'няма', 'има', 'само', 'може', 'биха', 'този', 'тази', 'тези',
      'един', 'една', 'едно', 'едни', 'дали', 'заради', 'върху', 'срещу', 'пред', 'към',
      'във', 'със', 'при', 'него', 'нея', 'тях', 'вече', 'още', 'много', 'малко', 'къде',
      'какво', 'кога', 'защо', 'така', 'нали', 'бяха', 'също', 'ще', 'да'
    ]);
    const tokens = (claimText || '').toLowerCase().match(/\d+(?:\s\d{3})+|\d{2,}|[\p{L}]{4,}/gu) || [];
    const pLower = (passageText || '').toLowerCase();
    const pWords = pLower.match(/[\p{L}]{4,}/gu) || [];
    const matched = new Set<string>();
    for (const w of tokens) {
      if (stopWords.has(w)) continue;
      if (pLower.includes(w)) {
        matched.add(w);
        continue;
      }
      const compactNum = w.replace(/\s+/g, '');
      if (/^\d{4,}$/.test(compactNum) && pLower.includes(compactNum)) {
        matched.add(compactNum);
        continue;
      }
      if (w.length >= 5 && /^[\p{L}]+$/u.test(w)) {
        const stem = w.slice(0, 5);
        const foundWord = pWords.find(pw => pw.startsWith(stem));
        if (foundWord) {
          matched.add(foundWord);
        }
      }
    }
    return Array.from(matched).slice(0, 6);
  }

  function sourceCategoryLabel(kind: string): { label: string; icon: string; cls: string } {
    switch ((kind || '').toLowerCase()) {
      case 'primary':
        return { label: currentLang === 'bg' ? 'Официална институция' : 'Primary / Gov', icon: '🏛️', cls: 'cat-primary' };
      case 'factcheck':
        return { label: currentLang === 'bg' ? 'Проверка на факти' : 'Fact Check', icon: '🛡️', cls: 'cat-factcheck' };
      case 'news':
        return { label: currentLang === 'bg' ? 'Новинарска медия' : 'News Media', icon: '📰', cls: 'cat-news' };
      case 'reference':
        return { label: currentLang === 'bg' ? 'Справочник' : 'Reference', icon: '📚', cls: 'cat-other' };
      default:
        return { label: currentLang === 'bg' ? 'Външен източник' : 'External Source', icon: '🌐', cls: 'cat-other' };
    }
  }

  async function handleResearchPerson(person: Person) {
    if (isJobActive('person-research', person.id)) return;
    startAiJob('person-research', person.id, person.canonical_name);
    copyToast = currentLang === 'bg' ? `⚡ Проучване: ${person.canonical_name}...` : `⚡ Researching ${person.canonical_name}...`;
    try {
      const updated = await researchPerson(person.id, person.canonical_name);
      if (detail?.people) {
        detail.people = detail.people.map(p => p.id === person.id ? { ...p, ...updated } : p);
      }
      finishAiJob('person-research', person.id, currentLang === 'bg' ? `✓ Справката за ${person.canonical_name} е готова` : `✓ Report ready: ${person.canonical_name}`, true);
      copyToast = currentLang === 'bg' ? `✓ Готова справка: ${person.canonical_name}` : `✓ Researched: ${person.canonical_name}`;
    } catch {
      finishAiJob('person-research', person.id, currentLang === 'bg' ? 'Грешка при проучване' : 'Research error', false);
      copyToast = currentLang === 'bg' ? 'Грешка при проучване' : 'Research error';
    } finally {
      setTimeout(() => (copyToast = null), 3000);
    }
  }

  async function handleRemoveThisVideo() {
    if (!detail) return;
    const vid = detail.video.id;
    onClose();
    if (onDiscard) {
      onDiscard(vid);
    } else if (onDeleteVideo) {
      onDeleteVideo(vid);
    } else {
      await deleteVideo(vid);
    }
  }

  async function handleSaveTranscript() {
    if (!detail || isSavingTranscript) return;
    isSavingTranscript = true;
    try {
      detail = await updateTranscript(detail.video.id, editedText, true);
      isEditingTranscript = false;
      copyToast = currentLang === 'bg' ? '✓ Стенограмата е запазена!' : '✓ Transcript saved!';
    } finally {
      isSavingTranscript = false;
      setTimeout(() => (copyToast = null), 2500);
    }
  }

  async function handleCopyTranscript() {
    if (!detail?.transcript) return;
    const txt = detail.transcript.edited_text || detail.transcript.text;
    await navigator.clipboard.writeText(txt);
    copyToast = currentLang === 'bg' ? '✓ Копирано в клипборда!' : '✓ Copied to clipboard!';
    setTimeout(() => (copyToast = null), 2500);
  }

  async function handleWarmStartRedact() {
    if (!detail || isRetranscribing) return;
    isRetranscribing = true;
    copyToast = currentLang === 'bg' ? '⚡ Преработка с речника и AI...' : '⚡ Running warm-start redaction...';
    try {
      const res = await retranscribeVideo(detail.video.id, true, true);
      if (res?.corrected_text && detail.transcript) {
        detail.transcript.edited_text = res.corrected_text;
        editedText = res.corrected_text;
      }
      copyToast = currentLang === 'bg' ? '✓ Преработено с речника и AI!' : '✓ Redacted with lexicon & AI!';
      await loadDetail(detail.video.id);
    } catch {
      copyToast = currentLang === 'bg' ? 'Грешка при преработка' : 'Redaction error';
    } finally {
      isRetranscribing = false;
      setTimeout(() => (copyToast = null), 3000);
    }
  }

  async function handleFullRetranscribe() {
    if (!detail?.video?.id || isJobActive('whisper', detail.video.id)) return;
    const vid = detail.video.id;
    startAiJob('whisper', vid, `Whisper: ${vid}`);
    copyToast = currentLang === 'bg' ? '🔄 Изпратено за Whisper...' : '🔄 Whisper queued...';
    try {
      await retranscribeVideo(vid, false, true);
      finishAiJob('whisper', vid, currentLang === 'bg' ? `✓ Whisper завърши за ${vid}` : `✓ Whisper completed for ${vid}`, true);
      copyToast = currentLang === 'bg' ? '✓ Whisper задачата е стартирана!' : '✓ Whisper task queued!';
    } catch {
      finishAiJob('whisper', vid, currentLang === 'bg' ? 'Грешка при Whisper задача' : 'Whisper queue error', false);
      copyToast = currentLang === 'bg' ? 'Грешка при Whisper задача' : 'Whisper queue error';
    } finally {
      setTimeout(() => (copyToast = null), 3000);
    }
  }

  async function handleAiCheckAndCopy() {
    if (!detail?.transcript || isAiChecking) return;
    isAiChecking = true;
    copyToast = currentLang === 'bg' ? 'AI проверка на текста...' : 'Running AI spellcheck...';
    try {
      const res = await aiCheckTranscript(detail.video.id);
      if (res && res.corrected_text) {
        detail.transcript.edited_text = res.corrected_text;
        editedText = res.corrected_text;
        copyToast = currentLang === 'bg' ? '✓ Коригирано с AI!' : '✓ AI corrected!';
      }
    } catch {
      copyToast = currentLang === 'bg' ? 'Грешка при AI проверка' : 'AI check error';
    } finally {
      isAiChecking = false;
      setTimeout(() => (copyToast = null), 3000);
    }
  }

  async function handleGoogleAiFix() {
    if (!detail?.video?.id || isJobActive('google-fix', detail.video.id)) return;
    const vid = detail.video.id;
    startAiJob('google-fix', vid, `AI Поправка: ${vid}`);
    copyToast = currentLang === 'bg' ? '✨ AI поправка на готовия транскрипт...' : '✨ Refining transcript with AI...';
    try {
      const res = await googleAiFixTranscript(vid);
      if (res && res.corrected_text) {
        if (detail && detail.video.id === vid && detail.transcript) {
          detail.transcript.edited_text = res.corrected_text;
          editedText = res.corrected_text;
        }
        finishAiJob('google-fix', vid, currentLang === 'bg' ? `✓ AI поправката за ${vid} е готова!` : `✓ AI fix ready for ${vid}!`, true);
        copyToast = currentLang === 'bg' ? '✓ Поправено с AI!' : '✓ Corrected with AI!';
      }
    } catch {
      finishAiJob('google-fix', vid, currentLang === 'bg' ? 'Грешка при AI поправка' : 'AI fix error', false);
      copyToast = currentLang === 'bg' ? 'Грешка при AI поправка' : 'AI fix error';
    } finally {
      setTimeout(() => (copyToast = null), 3000);
    }
  }

  async function handleGoogleAudioTranscribe() {
    if (!detail?.video?.id || isJobActive('google-audio', detail.video.id)) return;
    const vid = detail.video.id;
    startAiJob('google-audio', vid, `Google AI: ${vid}`);
    copyToast = currentLang === 'bg' ? '🎙️ Google AI аудио транскрибиране...' : '🎙️ Google AI transcribing audio...';
    try {
      const res = await googleAudioTranscribe(vid);
      if (res && res.corrected_text) {
        if (detail && detail.video.id === vid) {
          if (!detail.transcript) {
            detail.transcript = {
              video_id: vid,
              source: 'google_audio',
              engine: 'google_web_speech',
              model: null,
              language: 'bg',
              quality: null,
              text: res.corrected_text,
              segments: [],
              edited_text: res.corrected_text,
            };
          } else {
            detail.transcript.edited_text = res.corrected_text;
          }
          editedText = res.corrected_text;
          await loadDetail(vid);
        }
        finishAiJob('google-audio', vid, currentLang === 'bg' ? `✓ Google AI аудио за ${vid} е готово!` : `✓ Google AI audio ready for ${vid}!`, true);
        copyToast = currentLang === 'bg' ? '✓ Google AI транскрипцията е готова!' : '✓ Google AI transcription ready!';
      }
    } catch {
      finishAiJob('google-audio', vid, currentLang === 'bg' ? 'Грешка при Google AI транскрибиране' : 'Google AI transcription error', false);
      copyToast = currentLang === 'bg' ? 'Грешка при Google AI транскрибиране' : 'Google AI transcription error';
    } finally {
      setTimeout(() => (copyToast = null), 3000);
    }
  }

  function togglePlayPause() {
    if (!videoElement) return;
    if (videoElement.paused) {
      videoElement.play();
      isPlaying = true;
    } else {
      videoElement.pause();
      isPlaying = false;
    }
  }

  function seekRelative(seconds: number) {
    if (!videoElement) return;
    videoElement.currentTime = Math.max(0, videoElement.currentTime + seconds);
  }

  function handleDrawerScroll(e: Event) {
    const target = e.target as HTMLElement;
    if (!target) return;
    if (videoContainerRef) {
      const rect = videoContainerRef.getBoundingClientRect();
      const parentRect = target.getBoundingClientRect();
      isVideoOutOfView = rect.bottom < parentRect.top + 80;
    } else {
      isVideoOutOfView = target.scrollTop > 100;
    }
  }

  function formatDuration(sec: number | null): string {
    if (!sec) return '00:00';
    const m = Math.floor(sec / 60);
    const s = Math.floor(sec % 60);
    return `${m.toString().padStart(2, '0')}:${s.toString().padStart(2, '0')}`;
  }

  function formatViews(v: number | null): string {
    if (!v) return '0';
    if (v >= 1_000_000) return `${(v / 1_000_000).toFixed(1)}M`;
    if (v >= 1_000) return `${(v / 1_000).toFixed(1)}K`;
    return v.toString();
  }

  function formatTime(ms: number): string {
    const totalSec = Math.floor(ms / 1000);
    const m = Math.floor(totalSec / 60);
    const s = Math.floor(totalSec % 60);
    return `${m.toString().padStart(2, '0')}:${s.toString().padStart(2, '0')}`;
  }

  function handleSegmentEdit(seg: any, newTxt: string) {
    seg.text = newTxt;
    editedText = transcriptDisplaySegments.map((s) => s.text).join(' ');
  }

  async function toggleTranscriptEditing() {
    if (isEditingTranscript) {
      await handleSaveTranscript();
    } else {
      if (!editedText) {
        editedText = detail?.transcript?.edited_text || detail?.transcript?.text || '';
      }
      isEditingTranscript = true;
    }
  }

  async function handleOpenDisambiguate(person: Person) {
    disambiguatingPerson = person;
    disambiguateTargetPersonId = person.id;
    disambiguateNewName = person.canonical_name;
    disambiguateRole = person.role || '';
    disambiguateTags = (person.tags || []).join(', ');
    disambiguateEntityType = (person.entity_type as EntityType) || 'person';
    try {
      availablePeople = await fetchPeople();
    } catch {}
  }

  async function handleConfirmDisambiguation() {
    if (!detail || !disambiguatingPerson) return;
    try {
      if (disambiguateTargetPersonId && disambiguateTargetPersonId !== disambiguatingPerson.id) {
        await reassignVideoPerson(detail.video.id, disambiguatingPerson.id, {
          new_person_id: disambiguateTargetPersonId,
        });
      } else if (
        (disambiguateNewName.trim() && disambiguateNewName.trim() !== disambiguatingPerson.canonical_name) ||
        disambiguateEntityType !== (disambiguatingPerson.entity_type || 'person') ||
        disambiguateRole.trim() !== (disambiguatingPerson.role || '')
      ) {
        const tagsArr = disambiguateTags.split(',').map((t) => t.trim().replace(/^\[|\]$/g, '')).filter(Boolean);
        await reassignVideoPerson(detail.video.id, disambiguatingPerson.id, {
          canonical_name: disambiguateNewName.trim() || disambiguatingPerson.canonical_name,
          role: disambiguateRole.trim() || undefined,
          tags: tagsArr,
          entity_type: disambiguateEntityType,
        });
      }
      const fresh = await fetchVideoDetail(detail.video.id);
      detail.people = fresh.people;
      disambiguatingPerson = null;
      copyToast = currentLang === 'bg' ? '✓ Лицето е коригирано!' : '✓ Person updated!';
      setTimeout(() => (copyToast = null), 2500);
    } catch {
      copyToast = currentLang === 'bg' ? 'Грешка при пренасочване' : 'Reassignment error';
      setTimeout(() => (copyToast = null), 2500);
    }
  }

  async function handleRemovePersonFromVideo(person: Person) {
    if (!detail) return;
    if (!confirm(currentLang === 'bg' ? `Премахване на ${person.canonical_name} от това видео?` : `Remove ${person.canonical_name}?`)) return;
    try {
      await removePersonFromVideo(detail.video.id, person.id);
      detail.people = detail.people.filter((p) => p.id !== person.id);
    } catch {}
  }

  async function handleOpenGoogleAiResearch(query: string, mode: 'research' | 'refine' | 'factcheck' = 'research') {
    if (!query.trim() || isGoogleAiQuerying) return;
    isGoogleAiQuerying = true;
    copyToast = currentLang === 'bg' ? '🌐 Google AI проучване...' : '🌐 Running Google AI research...';
    try {
      const res = await queryGoogleAi(query, mode);
      googleAiModalData = res;
    } catch {
      copyToast = currentLang === 'bg' ? 'Грешка при Google AI проучване' : 'Google AI research error';
    } finally {
      isGoogleAiQuerying = false;
      setTimeout(() => (copyToast = null), 3000);
    }
  }

  function handleApplyGoogleAiRefinedText(text: string) {
    if (!detail?.transcript) return;
    detail.transcript.edited_text = text;
    editedText = text;
    googleAiModalData = null;
    copyToast = currentLang === 'bg' ? '✓ Текстът е приложен!' : '✓ Text applied!';
    setTimeout(() => (copyToast = null), 2500);
  }

  async function handleTriggerClaimVerify(claimText: string) {
    if (!detail) return;
    const vid = detail.video.id;
    startAiJob('video-research', vid, currentLang === 'bg' ? `Допълнителна проверка: ${vid}` : `Deep check: ${vid}`);
    copyToast = currentLang === 'bg' ? '🔍 Проверка в мрежата...' : '🔍 Searching online...';
    try {
      await logDecision(vid, 'research_more', claimText);
      finishAiJob('video-research', vid, currentLang === 'bg' ? `✓ Твърдението е поставено в опашката за проверка` : `✓ Enqueued for web verification`, true);
      copyToast = currentLang === 'bg' ? '✓ Изпратено за проверка!' : '✓ Enqueued for web research!';
    } catch {
      finishAiJob('video-research', vid, currentLang === 'bg' ? 'Грешка при изпращане' : 'Verification error', false);
      copyToast = currentLang === 'bg' ? 'Грешка при изпращане' : 'Verification error';
    } finally {
      setTimeout(() => (copyToast = null), 3000);
    }
  }

  async function handleCopyCitation(cwith: ClaimWithEvidence) {
    if (!detail) return;
    const author = detail.creator?.handle ? `@${detail.creator.handle}` : (detail.video.creator_handle ? `@${detail.video.creator_handle}` : 'неизвестен');
    const startS = Math.round((cwith.claim.start_ms || 0) / 1000);
    const endS = Math.round((cwith.claim.end_ms || 0) / 1000);
    const citation = `"${cwith.claim.text_bg}" (Видео ID: ${detail.video.id}, Автор: ${author}, Времеви отрязък: ${startS}s-${endS}s, Източник: ${claimSourceLabel(claimSource(cwith.claim.text_bg))}, Оценка: ${cwith.claim.label || cwith.claim.verdict || 'Непроверено'})`;
    await navigator.clipboard.writeText(citation);
    copyToast = currentLang === 'bg' ? '✓ Цитатът е копиран!' : '✓ Citation copied!';
    setTimeout(() => (copyToast = null), 2500);
  }

  async function handleToggleWatchlist() {
    if (!detail?.creator || isTogglingWatchlist) return;
    isTogglingWatchlist = true;
    const nextState = !detail.creator.watchlist;
    try {
      await setCreatorWatchlist(detail.creator.handle, nextState);
      detail.creator.watchlist = nextState;
      copyToast = nextState
        ? (currentLang === 'bg' ? '✓ Добавен в наблюдение' : '✓ Added to watchlist')
        : (currentLang === 'bg' ? '✓ Премахнат от наблюдение' : '✓ Removed from watchlist');
    } catch {
      copyToast = currentLang === 'bg' ? 'Грешка при обновяване' : 'Watchlist update error';
    } finally {
      isTogglingWatchlist = false;
      setTimeout(() => (copyToast = null), 2500);
    }
  }

  function computePersonCredibility(person: Person) {
    const pAny = person as any;
    const dbContradicted = pAny.contradicted_count ?? 0;
    const dbSupported = pAny.supported_count ?? 0;
    const dbTotal = pAny.total_claims ?? 0;

    const nameLower = person.canonical_name.toLowerCase();
    const related = (detail?.claims || []).filter((c) => {
      const sp = (c.claim.speaker || '').toLowerCase();
      const attr = (c.claim.attribution || []).some((a) => (a.actor || '').toLowerCase().includes(nameLower));
      return sp.includes(nameLower) || attr;
    });

    const hasContradicted = related.some((c) => (c.claim.verdict || '').toUpperCase() === 'CONTRADICTED') || dbContradicted > 0;
    const hasSupported = related.some((c) => (c.claim.verdict || '').toUpperCase() === 'SUPPORTED') || dbSupported > 0;

    let score = 65;
    let level = 'neutral';
    let label = 'Споменато лице';
    let levelLabel = 'Неутрален';
    let badgeClass = 'cred-neutral';

    if (hasContradicted) {
      score = Math.max(15, 35 - dbContradicted * 5);
      level = 'high-risk';
      label = '⚠️ Повишен риск от дезинформация';
      levelLabel = 'Нисък рейтинг на достоверност';
      badgeClass = 'cred-low';
    } else if (hasSupported) {
      score = Math.min(95, 80 + dbSupported * 3);
      level = 'supported';
      label = '✅ Висока достоверност';
      levelLabel = 'Потвърдени факти';
      badgeClass = 'cred-high';
    } else if (related.length > 0 || dbTotal > 0) {
      score = 50;
      level = 'unverifiable';
      label = 'ℹ️ Изисква проверка';
      levelLabel = 'Непроверени твърдения';
      badgeClass = 'cred-moderate';
    }

    return {
      score,
      level,
      label,
      levelLabel,
      badgeClass,
      stats: {
        total: related.length + dbTotal,
        contradicted: (related.filter(c => (c.claim.verdict || '').toUpperCase() === 'CONTRADICTED').length) + dbContradicted,
        supported: (related.filter(c => (c.claim.verdict || '').toUpperCase() === 'SUPPORTED').length) + dbSupported,
      }
    };
  }

  function formatRiskAssessment(ra: any) {
    const level = ra.risk_level || 'unverified';
    const conf = ra.confidence ?? 0.2;
    const confPercent = Math.round(conf * 100);
    const confLevel = ra.confidence_level || (conf >= 0.7 ? 'high' : conf >= 0.35 ? 'moderate' : 'low');

    let label = '⚪ Непроверен профил';
    let badgeClass = 'risk-unverified';
    if (level === 'high') {
      label = currentLang === 'bg' ? '🔴 Висок риск' : '🔴 High Risk';
      badgeClass = 'risk-high';
    } else if (level === 'moderate') {
      label = currentLang === 'bg' ? '🟡 Умерен риск' : '🟡 Moderate Risk';
      badgeClass = 'risk-moderate';
    } else if (level === 'low') {
      label = currentLang === 'bg' ? '🟢 Нисък риск' : '🟢 Low Risk';
      badgeClass = 'risk-low';
    } else {
      label = currentLang === 'bg' ? '⚪ Непроверен / Недостатъчно данни' : '⚪ Unverified / Insufficient Data';
    }

    const confText = confLevel === 'high'
      ? (currentLang === 'bg' ? 'Висока увереност' : 'High confidence')
      : confLevel === 'moderate'
        ? (currentLang === 'bg' ? 'Умерена увереност' : 'Moderate confidence')
        : (currentLang === 'bg' ? 'Нисък доверителен интервал' : 'Low confidence');

    return {
      level,
      label,
      badgeClass,
      confidenceScore: conf,
      confidenceLevel: confLevel,
      confidencePercent: confPercent,
      confidenceClass: `conf-${confLevel}`,
      confidenceText: confText,
      confidenceReason: ra.confidence_reason || '',
      rationale: ra.rationale || '',
      webSources: ra.web_sources || [],
      localStats: ra.local_stats || { total_videos: 1, flagged_videos: 0, contradicted_claims: 0, supported_claims: 0 },
      lastCalculatedAt: ra.last_calculated_at || null,
    };
  }

  function computeAuthorRisk(creator: any) {
    if (creator?.risk_assessment) {
      return formatRiskAssessment(creator.risk_assessment);
    }
    if (creator?.risk_assessment_json) {
      try {
        const parsed = JSON.parse(creator.risk_assessment_json);
        return formatRiskAssessment(parsed);
      } catch (e) {
      }
    }

    const flagged = creator?.flagged_count ?? 0;
    const totalVideos = creatorVideos.length || (creator?.total_videos ?? (detail?.video ? 1 : 0));
    const hasContradicted = detail?.claims?.some((c) => (c.claim.verdict || '').toUpperCase() === 'CONTRADICTED');
    const hasSupported = detail?.claims?.some((c) => (c.claim.verdict || '').toUpperCase() === 'SUPPORTED');

    let level: 'high' | 'moderate' | 'low' | 'unverified' = 'unverified';
    let label = currentLang === 'bg' ? '⚪ Непроверен профил' : '⚪ Unverified Profile';
    let badgeClass = 'risk-unverified';
    let confidenceScore = 0.20;
    let confidenceLevel: 'low' | 'moderate' | 'high' = 'low';
    let confidencePercent = 20;
    let confidenceText = currentLang === 'bg' ? 'Нисък доверителен интервал' : 'Low confidence';
    let confidenceReason = currentLang === 'bg'
      ? `Малък обем данни: само ${totalVideos} видео в локалната база без външни уеб данни.`
      : `Small sample size: only ${totalVideos} video in local database without web data.`;
    let rationale = currentLang === 'bg'
      ? 'Липсват достатъчно записи или външни журналистически проверки за определяне на категоричен риск.'
      : 'Insufficient local records or external journalistic checks to determine conclusive risk.';

    if (flagged >= 3 || hasContradicted) {
      level = 'high';
      label = currentLang === 'bg' ? '🔴 Висок риск' : '🔴 High Risk';
      badgeClass = 'risk-high';
      confidenceScore = totalVideos >= 3 ? 0.70 : 0.40;
      confidenceLevel = totalVideos >= 3 ? 'high' : 'moderate';
      confidencePercent = Math.round(confidenceScore * 100);
      confidenceText = confidenceLevel === 'high'
        ? (currentLang === 'bg' ? 'Висока увереност' : 'High confidence')
        : (currentLang === 'bg' ? 'Умерен доверителен интервал' : 'Moderate confidence');
      confidenceReason = currentLang === 'bg'
        ? `Открито опровергано твърдение, но историята е ограничена до ${totalVideos} видео в базата.`
        : `Contradicted claim detected, but history is limited to ${totalVideos} video in database.`;
      rationale = currentLang === 'bg'
        ? 'Авторът е разпространил съдържание с опровергани твърдения.'
        : 'Author has distributed content containing contradicted claims.';
    } else if (flagged > 0) {
      level = 'moderate';
      label = currentLang === 'bg' ? '🟡 Умерен риск' : '🟡 Moderate Risk';
      badgeClass = 'risk-moderate';
      confidenceScore = 0.35;
      confidenceLevel = 'moderate';
      confidencePercent = 35;
      confidenceText = currentLang === 'bg' ? 'Умерен доверителен интервал' : 'Moderate confidence';
      confidenceReason = currentLang === 'bg'
        ? `Базирано на ${flagged} предишно сигнализирано видео.`
        : `Based on ${flagged} previously flagged video.`;
      rationale = currentLang === 'bg'
        ? 'Авторът има предишни видеа с повишен риск в базата.'
        : 'Author has previously flagged content in the database.';
    } else if (totalVideos >= 3 && hasSupported) {
      level = 'low';
      label = currentLang === 'bg' ? '🟢 Нисък риск' : '🟢 Low Risk';
      badgeClass = 'risk-low';
      confidenceScore = 0.65;
      confidenceLevel = 'moderate';
      confidencePercent = 65;
      confidenceText = currentLang === 'bg' ? 'Умерена увереност' : 'Moderate confidence';
      confidenceReason = currentLang === 'bg'
        ? `Базирано на ${totalVideos} видеа в базата без регистрирани нарушения.`
        : `Based on ${totalVideos} videos in database without violations.`;
      rationale = currentLang === 'bg'
        ? 'Потвърдени надеждни твърдения в локалната база без нарушения.'
        : 'Confirmed supported claims in local database without violations.';
    }

    return {
      level,
      label,
      badgeClass,
      confidenceScore,
      confidenceLevel,
      confidencePercent,
      confidenceClass: `conf-${confidenceLevel}`,
      confidenceText,
      confidenceReason,
      rationale,
      webSources: [],
      localStats: {
        total_videos: totalVideos,
        flagged_videos: flagged,
        contradicted_claims: hasContradicted ? 1 : 0,
        supported_claims: hasSupported ? 1 : 0,
      },
      lastCalculatedAt: null,
    };
  }

  async function handleAssessAuthorWebRisk() {
    const handle = detail?.creator?.handle || detail?.video?.creator_handle;
    if (!handle || isAssessingRisk) return;
    isAssessingRisk = true;
    startAiJob('creator-risk', handle, currentLang === 'bg' ? `Проверка на автор: @${handle}` : `Creator check: @${handle}`);
    try {
      const res = await assessCreatorRisk(handle, detail?.creator?.display_name || detail?.video?.creator_display_name);
      if (detail) {
        if (!detail.creator) {
          detail.creator = {
            id: handle.toLowerCase(),
            handle,
            display_name: detail.video?.creator_display_name ?? null,
            region: null,
            follower_count: detail.video?.creator_follower_count ?? null,
            first_seen_at: new Date().toISOString(),
            watchlist: false,
            flagged_count: 0,
            expansion_depth: 0,
            risk_assessment: res,
            risk_assessment_json: JSON.stringify(res),
          };
        } else {
          detail.creator.risk_assessment = res;
          detail.creator.risk_assessment_json = JSON.stringify(res);
        }
      }
      finishAiJob('creator-risk', handle, currentLang === 'bg' ? `✓ Проверката на @${handle} завърши` : `✓ @${handle} check complete`, true);
    } catch (err) {
      finishAiJob('creator-risk', handle, currentLang === 'bg' ? `Грешка при проверка на @${handle}` : `Error checking @${handle}`, false);
      console.error('Failed to assess author risk via web:', err);
    } finally {
      isAssessingRisk = false;
    }
  }

  function stanceClass(stance: string): string {
    switch (stance.toLowerCase()) {
      case 'contradicts':
        return 'stance-contradicts';
      case 'supports':
        return 'stance-supports';
      default:
        return 'stance-neutral';
    }
  }

  function stanceLabel(stance: string): string {
    switch (stance.toLowerCase()) {
      case 'contradicts':
        return t.stanceContradicts;
      case 'supports':
        return t.stanceSupports;
      default:
        return t.stanceNeutral;
    }
  }

  const videoSrc = $derived(
    detail?.video.id ? `${getCoreUrl()}/api/media/${detail.video.id}/video.mp4` : ''
  );

  function generateAllInformationText(): string {
    if (!detail) return '';
    const lines: string[] = [];
    const bg = currentLang === 'bg';

    lines.push('╔══════════════════════════════════════════════════════════════════════════════╗');
    lines.push(bg ? '║               ПРЕГЛЕД НА СЪДЪРЖАНИЕТО И ДОКАЗАТЕЛСТВАТА                      ║' : '║                  CONTENT & EVIDENCE REVIEW REPORT                            ║');
    lines.push('╠══════════════════════════════════════════════════════════════════════════════╣');
    lines.push(`║ Видео ID:     ${detail.video.id}`);
    lines.push(`║ TikTok URL:   ${detail.video.url}`);
    lines.push(`║ Автор:        @${detail.video.creator_handle || 'неизвестен'}${detail.video.creator_display_name ? ` (${detail.video.creator_display_name})` : ''}`);
    lines.push(`║ Показвания:   ${(detail.video.views || 0).toLocaleString()} | Времетраене: ${formatDuration(detail.video.duration_s)}`);
    if (detail.video.posted_at) {
      lines.push(`║ Публикувано:  ${detail.video.posted_at}`);
    }
    if (detail.video.caption) {
      lines.push(`║ Описание:     ${detail.video.caption.trim().replace(/\n/g, ' ')}`);
    }
    lines.push('╚══════════════════════════════════════════════════════════════════════════════╝');
    lines.push('');

    lines.push(bg ? '┌── 🎙️ СТЕНОГРАМА И ТЕКСТ ОТ ВИДЕОТО ──────────────────────────────────────────' : '┌── 🎙️ TRANSCRIPT & SPEECH RECOGNITION ──────────────────────────────────────');
    if (detail.transcript) {
      const qPct = Math.round((detail.transcript.quality || 0) * 100);
      lines.push(`│ Източник: ${detail.transcript.source} | Език: ${detail.transcript.language} | Качество: ${qPct}%`);
      const txt = detail.transcript.edited_text || detail.transcript.text;
      lines.push('│');
      lines.push('│ [Пълен текст на стенограмата]:');
      for (const l of (txt || '').split('\n')) {
        if (l.trim()) lines.push(`│   ${l.trim()}`);
      }
      if (detail.transcript.segments && detail.transcript.segments.length > 0) {
        lines.push('│');
        lines.push('│ [Сегменти по таймкод]:');
        for (const seg of detail.transcript.segments) {
          const tStart = formatTime(seg.start_ms);
          const tEnd = formatTime(seg.end_ms);
          const spk = seg.speaker ? ` (${seg.speaker})` : '';
          lines.push(`│   • [${tStart} - ${tEnd}]${spk}: ${seg.text}`);
        }
      }
    } else {
      lines.push(bg ? '│ (Няма налична стенограма)' : '│ (No transcript available)');
    }

    if (ocrItems && ocrItems.length > 0) {
      lines.push('│');
      lines.push(bg ? `│ ── 🖥️ ТЕКСТ НА ЕКРАНА (OCR) (${ocrItems.length} засечени кадъра) ────────────────` : `│ ── 🖥️ ON-SCREEN OCR TEXT (${ocrItems.length} frames) ──────────────────────────`);
      for (const o of ocrItems) {
        const tFrame = formatTime(o.frame_ms);
        const confPct = Math.round((o.confidence ?? 0) * 100);
        lines.push(`│   • [${tFrame}] (${confPct}% увереност): ${o.text}`);
      }
    }
    lines.push('└─────────────────────────────────────────────────────────────────────────────');
    lines.push('');

    lines.push(bg ? '┌── 🛡️ ПРОВЕРКА ЗА МАНИПУЛИРАНО ВИДЕО И ГЛАС (AEGIS) ─────────────────────────' : '┌── 🛡️ AEGIS DEEPFAKE & SYNTHETIC FORENSICS ─────────────────────────────────');
    if (detail.aegis_deepfake) {
      const df = detail.aegis_deepfake;
      const vNorm = normalizeAegisVerdict(df.overall_verdict, df.audio?.synthetic, df.video?.synthetic);
      let verdictStr = '✅ ИСТИНСКИ ЗАПИС';
      if (vNorm === 'deepfake') verdictStr = '🚨 МАНИПУЛИРАН ЗАПИС';
      else if (vNorm === 'suspicious') verdictStr = '⚠️ СЪМНИТЕЛЕН ЗАПИС';

      const audioRiskPct = Math.round((df.audio?.score ?? 0) * 100);
      const videoRiskPct = Math.round((df.video?.score ?? 0) * 100);
      const maxRisk = Math.max(audioRiskPct, videoRiskPct);

      lines.push(`│ Заключение:   ${verdictStr} | Риск от манипулация: ${maxRisk}%`);
      if (df.summary_bg) {
        lines.push(`│ Обобщение:    ${df.summary_bg.trim().replace(/\n/g, ' ')}`);
      }
      lines.push('│');
      lines.push('│ 🎵 Звук и глас');
      const aClass = df.audio.classification === 'cloned_voice'
        ? '🎭 Клониран глас'
        : df.audio.classification === 'robotic_tts'
        ? '🤖 Генериран глас'
        : '🎙️ Естествен глас';
      lines.push(`│   • Вид:           ${aClass} (Изкуствен сигнал: ${audioRiskPct}%)`);
      if (df.audio.target_person && (df.audio.target_confidence ?? 0) >= 0.8) {
        lines.push(`│   • Приписан глас: 🎯 ${df.audio.target_person} (${Math.round((df.audio.target_confidence ?? 0) * 100)}% точност)`);
      }
      if (df.audio.anomalies && df.audio.anomalies.length > 0) {
        lines.push(`│   • Отклонения:    ${df.audio.anomalies.map(a => `⚡ ${a}`).join(', ')}`);
      }
      if (df.audio.metrics) {
        const m = df.audio.metrics;
        const f0 = m.f0_mean_hz !== undefined ? `${m.f0_mean_hz} Hz` : 'Н/Д';
        const f0Std = (m.f0_std_hz ?? m.f0_std) !== undefined ? `${m.f0_std_hz ?? m.f0_std} Hz` : 'Н/Д';
        const vc = m.spectral_cutoff_khz !== undefined
          ? `${m.spectral_cutoff_khz} kHz`
          : (m.vocoder_cutoff_freq ?? m.vocoder_cutoff_ratio) !== undefined
          ? `${m.vocoder_cutoff_freq ?? m.vocoder_cutoff_ratio}`
          : 'Н/Д';
        lines.push(`│   • Показатели:    F0: ${f0} (±${f0Std}) | Честотен праг: ${vc}`);
      }
      lines.push('│');
      lines.push('│ 🎬 Картина и лице');
      lines.push(`│   • Вид:           ${df.video.synthetic ? '⚠️ Обработено / генерирано видео' : '✅ Реално заснето видео'} (Изкуствен сигнал: ${videoRiskPct}%)`);
      if (df.video.metrics) {
        const vm = df.video.metrics;
        const fft = (vm.mean_fft_spectrum_ratio ?? vm.fft_ratio ?? vm.fft_anomaly_score) !== undefined
          ? (vm.mean_fft_spectrum_ratio ?? vm.fft_ratio ?? vm.fft_anomaly_score)
          : 'Н/Д';
        const lap = (vm.mean_laplacian_variance ?? vm.laplacian_var ?? vm.laplacian_variance) !== undefined
          ? (vm.mean_laplacian_variance ?? vm.laplacian_var ?? vm.laplacian_variance)
          : 'Н/Д';
        lines.push(`│   • Показатели:    FFT спектър: ${fft} | Рязкост (Laplacian): ${lap}`);
      }
      if (df.multimodal) {
        lines.push('│');
        lines.push('│ 🔗 Мултимодален AV-Sync анализ');
        const syncScore = df.multimodal.av_sync_score !== undefined && df.multimodal.av_sync_score !== null
          ? `${Math.round(Number(df.multimodal.av_sync_score) * 100)}%`
          : 'Н/Д';
        const mismatchStr = df.multimodal.av_sync_mismatch ? '⚠️ Засечено разминаване на устни и говор' : '✅ Нормален синхрон';
        lines.push(`│   • AV-Sync статус: ${mismatchStr} (Синхрон: ${syncScore})`);
      }
    } else {
      lines.push(bg ? '│ (Все още не е сканирано с AEGIS)' : '│ (Not scanned with AEGIS yet)');
    }
    lines.push('└─────────────────────────────────────────────────────────────────────────────');
    lines.push('');

    lines.push(bg ? `┌── ⚖️ ПРОВЕРЕНИ ТВЪРДЕНИЯ И ДОКАЗАТЕЛСТВА (${detail.claims.length}) ──────────────────────────` : `┌── ⚖️ VERIFIED CLAIMS & EVIDENCE (${detail.claims.length}) ──────────────────────────────`);
    if (detail.claims.length === 0) {
      lines.push(bg ? '│ (Няма открити проверими твърдения)' : '│ (No claims detected)');
    } else {
      detail.claims.forEach((cwith, idx) => {
        const c = cwith.claim;
        const vText = formatVerdictDisplay(c.verdict, c.label, currentLang);
        const tStart = formatTime(c.start_ms || 0);
        const tEnd = formatTime(c.end_ms || 0);
        const src = claimSourceLabel(claimSource(c.text_bg));
        lines.push('│');
        lines.push(`│ ┌─ ТВЪРДЕНИЕ Т${idx + 1} [${(c.claim_type || 'fact').toUpperCase()}] [${src}] ───────────────────────────────`);
        lines.push(`│ │ Твърдение:   "${c.text_bg}"`);
        lines.push(`│ │ Оценка:      ${vText}`);
        lines.push(`│ │ Интервал:    ${tStart} - ${tEnd} | Достоверност: ${Math.round((c.checkworthiness || 0) * 100)}% | Сила: ${Math.round((c.evidence_strength || 0) * 100)}%`);
        if (c.reasoning_bg) {
          const rb = parseReasoningBreakdown(c.reasoning_bg, c.verdict);
          if (rb.items.length > 0) {
            lines.push('│ │ Мотиви и оценка по елементи:');
            for (const it of rb.items) {
              const sym = it.status === 'supported' ? '✓ [ПОТВЪРДЕНО]' : it.status === 'contradicted' ? '✗ [ОПРОВЕРГАНО]' : '? [НЕПРОВЕРИМО]';
              const prefix = it.statement ? `${it.statement} — ` : '';
              lines.push(`│ │   • ${sym} ${prefix}${it.detail}`);
            }
            if (rb.summary) {
              lines.push(`│ │ 📌 Обобщение: ${rb.summary}`);
            }
            for (const note of rb.extraNotes) {
              lines.push(`│ │ ℹ️ ${note}`);
            }
          } else {
            lines.push(`│ │ 📌 Мотиви и обобщение: ${rb.summary || c.reasoning_bg}`);
          }
        }
        if (c.attribution && c.attribution.length > 0) {
          for (const attr of c.attribution) {
            const actor = attr.actor || attr.asserted_part || 'Говорещият';
            const about = attr.about || attr.attributed_part || '';
            lines.push(`│ │ Приписване:  ${actor} каза: ${about}`);
          }
        }
        const validEv = cwith.evidence.filter((ev) => !isJunkSource(ev));
        if (validEv.length > 0) {
          lines.push('│ │');
          lines.push(`│ │ ── Доказателствена база (${validEv.length} външни източника) ────────────────────`);
          for (const ev of validEv) {
            const stLabel = ev.stance === 'supports' ? '✓ ПОТВЪРЖДАВА' : ev.stance === 'contradicts' ? '✗ ОПРОВЕРГАВА' : 'ℹ КОНТЕКСТ';
            const sName = ev.source_name || ev.source_kind;
            const sDate = ev.published_at ? ` (${ev.published_at.substring(0, 10)})` : '';
            const simPct = ev.similarity ? ` [${Math.round(ev.similarity * 100)}% сходство]` : '';
            lines.push(`│ │   • [${stLabel}] ${sName}${sDate}${simPct}`);
            const rawTitle = (ev as any).title;
            if (rawTitle) {
              const cleanTitle = rawTitle.replace(/<[^>]*>/g, '').trim();
              lines.push(`│ │     Заглавие: ${cleanTitle}`);
            }
            const snippet = ev.passage || (ev as any).snippet;
            if (snippet) {
              const cleanSnippet = snippet.replace(/<[^>]*>/g, '').trim().replace(/\n/g, ' ');
              lines.push(`│ │     Цитат:    "${cleanSnippet}"`);
            }
            const url = ev.source_url || (ev as any).url;
            if (url) {
              lines.push(`│ │     URL:      ${url}`);
            }
          }
        } else {
          lines.push('│ │ (Няма намерени външни доказателства)');
        }
        lines.push('│ └──────────────────────────────────────────────────────────────────────────');
      });
    }
    lines.push('└─────────────────────────────────────────────────────────────────────────────');
    lines.push('');

    if (detail.people && detail.people.length > 0) {
      lines.push(bg ? `┌── 👥 ЗАСЕЧЕНИ ЛИЧНОСТИ И ОБЩЕСТВЕНИ ФИГУРИ (${detail.people.length}) ──────────────────────` : `┌── 👥 DETECTED FIGURES & ENTITIES (${detail.people.length}) ────────────────────────────`);
      for (const p of detail.people) {
        const cred = computePersonCredibility(p);
        const eType = getEntityTypeInfo(p.entity_type).label;
        lines.push(`│ • ${p.canonical_name} [${eType}]`);
        if (p.role || (p as any).role_hint) {
          lines.push(`│   Длъжност:       ${p.role || (p as any).role_hint}`);
        }
        lines.push(`│   Достоверност:   ${cred.score}% (${cred.levelLabel})`);
        if (cred.stats && cred.stats.total > 0) {
          lines.push(`│   Статистика:     ${cred.stats.total} твърдения (✓ Потвърдени: ${cred.stats.supported}, ✗ Опровергани: ${cred.stats.contradicted})`);
        }
        if (p.tags && p.tags.length > 0) {
          lines.push(`│   Тагове:         ${p.tags.map(t => `#${t}`).join(', ')}`);
        }
        if (p.wikidata_id) {
          lines.push(`│   Wikidata ID:    ${p.wikidata_id} (https://www.wikidata.org/wiki/${p.wikidata_id})`);
        }
        if (p.summary) {
          lines.push(`│   Резюме:         ${p.summary.trim().replace(/\n/g, ' ')}`);
        }
        lines.push('│');
      }
      lines.push('└─────────────────────────────────────────────────────────────────────────────');
      lines.push('');
    }

    if (detail.creator || detail.video.creator_handle) {
      const creator = detail.creator || {
        id: (detail.video.creator_handle || '').toLowerCase(),
        handle: detail.video.creator_handle || '',
        display_name: detail.video.creator_display_name,
        follower_count: detail.video.creator_follower_count ?? null,
        region: 'BG',
        first_seen_at: new Date().toISOString(),
        flagged_count: 0,
        watchlist: false,
        expansion_depth: 0,
      };
      const authorRisk = computeAuthorRisk(creator);
      lines.push(bg ? '┌── 👤 ПРОФИЛ И ОЦЕНКА НА РИСКА НА АВТОРА ──────────────────────────────────' : '┌── 👤 CREATOR & REPUTATION RISK PROFILE ────────────────────────────────────');
      lines.push(`│ Автор:         @${creator.handle}${creator.display_name ? ` (${creator.display_name})` : ''}`);
      lines.push(`│ Последователи: ${(creator.follower_count || 0).toLocaleString()} | Регион: ${creator.region || 'BG'}`);
      lines.push(`│ Маркирани видеа: ${creator.flagged_count || 0} | Наблюдение: ${creator.watchlist ? '👁 ДА' : 'НЕ'}`);
      lines.push(`│ Рисков статус: ${authorRisk.label} | Системна увереност: ${authorRisk.confidencePercent}% (${authorRisk.confidenceText})`);
      lines.push(`│ Мотиви / Риск: ${authorRisk.rationale}`);
      if (authorRisk.webSources && authorRisk.webSources.length > 0) {
        lines.push('│');
        lines.push(`│ Открити уеб източници и проверки за автора (${authorRisk.webSources.length}):`);
        for (const ws of authorRisk.webSources) {
          lines.push(`│   • [${ws.domain}] ${ws.title} (${ws.url})`);
        }
      }
      if (creatorVideos && creatorVideos.length > 0) {
        lines.push('│');
        lines.push(`│ Други засечени видеа от автора в базата (${creatorVideos.length}):`);
        for (const ov of creatorVideos) {
          const vLabel = formatVerdictDisplay(ov.top_claim_verdict, ov.top_claim_label, currentLang);
          lines.push(`│   • [ID: ${ov.id}] 👁 ${(ov.views || 0).toLocaleString()} | ${vLabel}`);
          if (ov.top_claim_text) {
            lines.push(`│     "${ov.top_claim_text}"`);
          }
        }
      }
      lines.push('└─────────────────────────────────────────────────────────────────────────────');
      lines.push('');
    }

    lines.push(bg ? '┌── 📊 ТЕХНИЧЕСКИ МЕТАДАННИ ──────────────────────────────────────────────────' : '┌── 📊 TECHNICAL METADATA ───────────────────────────────────────────────────');
    lines.push(`│ Статус:       ${detail.video.state || 'UNKNOWN'}${detail.video.state_reason ? ` (${detail.video.state_reason})` : ''}`);
    lines.push(`│ Приоритет:    ${detail.video.lead_score ?? 'Н/Д'}`);
    lines.push(`│ Ангажираност: ${(detail.video.views || 0).toLocaleString()} гледания | ${(detail.video.likes || 0).toLocaleString()} харесвания | ${(detail.video.comments || 0).toLocaleString()} коментара | ${(detail.video.shares || 0).toLocaleString()} споделяния`);
    if (detail.video.phash) {
      lines.push(`│ Перцептивен хеш (pHash): ${detail.video.phash}`);
    }
    if (detail.video.hashtags && detail.video.hashtags.length > 0) {
      lines.push(`│ Хаштагове:    ${detail.video.hashtags.map(h => `#${h}`).join(' ')}`);
    }
    lines.push('└─────────────────────────────────────────────────────────────────────────────');

    return lines.join('\n');
  }

  const drawerTocItems = $derived.by((): TocItem[] => {
    if (!detail) return [];
    const bg = currentLang === 'bg';
    const items: TocItem[] = [
      {
        id: 'sec-transcript',
        label: bg ? 'Стенограма' : 'Transcript',
        badge: detail.transcript ? `${transcriptDisplaySegments.length}` : undefined,
      },
      {
        id: 'sec-aegis',
        label: bg ? 'Запис' : 'Media Check',
      },
      {
        id: 'sec-claims',
        label: bg ? 'Твърдения' : 'Claims',
        badge: `${detail.claims.length}`,
      },
    ];
    detail.claims.forEach((cwith, idx) => {
      const vc = verdictColorKey(cwith.claim.verdict);
      items.push({
        id: `claim-${cwith.claim.id}`,
        label: `Т${idx + 1}`,
        tone: vc === 'supported' ? 'green' : vc === 'contradicted' ? 'red' : vc === 'unverifiable' ? 'amber' : 'default',
      });
    });
    if (detail.people && detail.people.length > 0) {
      items.push({
        id: 'sec-people',
        label: bg ? 'Обекти' : 'Subjects',
        badge: `${detail.people.length}`,
      });
    }
    if (detail.creator || detail.video.creator_handle) {
      items.push({
        id: 'sec-creator',
        label: bg ? 'Автор' : 'Author',
      });
    }
    items.push({
      id: 'sec-metadata',
      label: bg ? 'Метаданни' : 'Metadata',
    });
    return items;
  });

  const previewParagraphs = $derived.by(() => {
    if (!previewModalData?.text) return [];
    return previewModalData.text
      .split(/\n{2,}/)
      .map((p) => p.trim())
      .filter(Boolean);
  });

  const previewTocItems = $derived.by((): TocItem[] => {
    if (previewParagraphs.length < 4) return [];
    const items: TocItem[] = [];
    const step = Math.max(2, Math.ceil(previewParagraphs.length / 6));
    for (let i = 0; i < previewParagraphs.length; i += step) {
      const sample = previewParagraphs[i].replace(/\s+/g, ' ').slice(0, 26).trim();
      items.push({
        id: `preview-para-${i}`,
        label: `§${i + 1} ${sample}${previewParagraphs[i].length > 26 ? '…' : ''}`,
      });
    }
    return items;
  });

  async function handleCopyAllInformation() {
    if (!detail) return;
    try {
      const fullText = generateAllInformationText();
      await navigator.clipboard.writeText(fullText);
      copyAllToast = currentLang === 'bg' ? '✓ Всичко е копирано!' : '✓ All copied!';
    } catch {
      copyAllToast = currentLang === 'bg' ? 'Грешка при копиране' : 'Copy failed';
    } finally {
      setTimeout(() => (copyAllToast = null), 2500);
    }
  }
</script>

<aside
  class="detail-drawer"
  class:widescreen-mode={widescreen.isActive}
  class:dual-pane-mode={widescreen.isDualPane}
  class:statement-grid-mode={widescreen.isStatementGrid}
  class:evidence-grid-mode={widescreen.isEvidenceGrid}
  class:eighteen-inch-mode={widescreen.isEighteenInch}
  class:ultrawide-mode={widescreen.isUltraWide}
  aria-label={t.drawerTitle}
>
  <div class="drawer-header">
    <div class="drawer-title-group">
      <h2 class="drawer-title">{t.drawerTitle}</h2>
      {#if detail}
        <span class="video-ref-id">ID: {detail.video.id}</span>
      {/if}
      {#if getActiveAiJobs().length > 0}
        <span class="active-jobs-badge" title={getActiveAiJobs().map(j => j.label).join('\n')}>
          ⚡ {getActiveAiJobs().length} {currentLang === 'bg' ? 'активни задачи' : 'active jobs'}
        </span>
      {/if}
    </div>
    <div class="drawer-header-actions">
      {#if copyAllToast}
        <span class="copy-toast">{copyAllToast}</span>
      {/if}
      {#if detail}
        <button
          class="btn btn-secondary btn-sm copy-all-btn"
          onclick={handleCopyAllInformation}
          title={currentLang === 'bg'
            ? 'Копирай цялата информация от всички секции в структуриран текстов формат'
            : 'Copy all information from all sections in structured text format'}
        >
          📋 {currentLang === 'bg' ? 'Копирай всичко' : 'Copy all'}
        </button>
      {/if}
      <button
        class="btn btn-danger btn-sm drawer-remove-btn"
        onclick={handleRemoveThisVideo}
        title={currentLang === 'bg' ? 'Премахни видеото от списъка (✕)' : 'Remove video (✕)'}
      >
        ✕ {currentLang === 'bg' ? 'Премахни видеото' : 'Remove video'}
      </button>
      <button class="close-btn" onclick={onClose} title={t.drawerClose} aria-label={t.drawerClose}>
        ✕
      </button>
    </div>
  </div>

  {#if isLoading}
    <div class="loading-state">
      <div class="spinner" aria-hidden="true"></div>
      <span>{currentLang === 'bg' ? 'Зареждане на детайлите...' : 'Loading video details...'}</span>
    </div>
  {:else if detail}
    <div class="drawer-toc-bar">
      <BookReaderToc
        variant="compact"
        items={drawerTocItems}
        scrollContainer={widescreen.isDualPane ? null : drawerBodyEl}
        title={currentLang === 'bg' ? 'Съдържание' : 'Contents'}
      />
    </div>
    <div class="drawer-body" bind:this={drawerBodyEl} onscroll={handleDrawerScroll}>
      <div class="drawer-col drawer-col-left" onscroll={handleDrawerScroll}>
      <div id="sec-transcript" class="section-card transcript-section">
        <div class="section-header">
          <div>
            <h3>{t.transcriptHeading}</h3>
            <span class="section-subhint">
              {currentLang === 'bg'
                ? 'Цветове: Червено = Опровергано, Зелено = Потвърдено, Жълто = Непроверимо'
                : 'Colors: Red = Contradicted, Green = Supported, Yellow = Unverifiable'}
            </span>
          </div>
          <div class="transcript-btn-group">
            {#if copyToast}
              <span class="copy-toast">{copyToast}</span>
            {/if}
            <button
              class="btn btn-secondary btn-sm"
              onclick={handleFullRetranscribe}
              disabled={isJobActive('whisper', detail.video.id)}
              title="Повторно гласово разпознаване от нулата с Whisper"
            >
              {isJobActive('whisper', detail.video.id) ? '...' : '🔄 Whisper'}
            </button>
            <button
              class="btn btn-primary btn-sm btn-ai-fix"
              onclick={handleGoogleAiFix}
              disabled={isJobActive('google-fix', detail.video.id)}
              title="Поправя фонетични грешки и имена в готовия транскрипт (без презаписване на клипборда)"
            >
              {isJobActive('google-fix', detail.video.id) ? '...' : '✨ AI Поправка'}
            </button>
            <button
              class="btn btn-secondary btn-sm btn-google-audio"
              onclick={handleGoogleAudioTranscribe}
              disabled={isJobActive('google-audio', detail.video.id)}
              title="Директно транскрибиране на гласа от видеото с Google AI Speech"
            >
              {isJobActive('google-audio', detail.video.id) ? '...' : '🎙️ Google AI'}
            </button>
            <button
              class="btn btn-secondary btn-sm"
              onclick={toggleTranscriptEditing}
              title="Ръчна редакция на текста на място с прослушване"
            >
              {isEditingTranscript ? (currentLang === 'bg' ? '✓ Запази' : '✓ Save') : '✏️ ' + (currentLang === 'bg' ? 'Редактирай' : 'Edit')}
            </button>
            <button
              class="btn btn-secondary btn-sm"
              onclick={handleCopyTranscript}
              title="Копирай транскрипта в клипборда"
            >
              📋 {currentLang === 'bg' ? 'Копирай' : 'Copy'}
            </button>
          </div>
        </div>

        <div class="media-section feed-mode" bind:this={videoContainerRef}>
          <div class="feed-top-meta">
            <div class="feed-creator-badge">
              <span class="creator-name">@{detail.video.creator_handle || 'tiktok_creator'}</span>
              {#if detail.video.views}
                <span class="creator-views">• {formatViews(detail.video.views)} гледания</span>
              {/if}
            </div>
            {#if detail.video.duration_s}
              <span class="feed-duration-pill">{formatDuration(detail.video.duration_s)}</span>
            {/if}
          </div>
          <div class="feed-video-container">
            <video
              bind:this={videoElement}
              src={videoSrc}
              poster={detail.video.thumbnail_url || undefined}
              controls
              playsinline
              ontimeupdate={handleTimeUpdate}
              onplay={() => {
                isPlaying = true;
                if (videoElement) {
                  registerPlaybackElement(videoElement, {
                    videoId: detail?.video?.id,
                    creatorHandle: detail?.video?.creator_handle,
                    caption: detail?.video?.caption,
                    thumbnailUrl: detail?.video?.thumbnail_url,
                  });
                  setPlaybackPlaying(true);
                }
              }}
              onpause={() => {
                isPlaying = false;
                setPlaybackPlaying(false);
              }}
              class="feed-vertical-player"
            >
              <track kind="captions" />
            </video>
          </div>
        </div>

        {#if detail.transcript}
          {#snippet ocrChip(o: OcrText)}
            <div
              class="ocr-chip"
              class:active={currentTimeMs >= o.frame_ms && currentTimeMs < o.frame_ms + 2500}
              onclick={() => seekToMs(o.frame_ms)}
              onkeydown={(e) => e.key === 'Enter' && seekToMs(o.frame_ms)}
              role="button"
              tabindex="0"
              title={`${currentLang === 'bg' ? 'Текст на екрана' : 'On-screen text'} · ${Math.round((o.confidence ?? 0) * 100)}%`}
            >
              <span class="ocr-chip-tag">🖥️ {formatTime(o.frame_ms)}</span>
              <span class="ocr-chip-text">{o.text}</span>
            </div>
          {/snippet}

          {#if ocrItems.length > 0}
            <div class="ocr-overview">
              <div class="ocr-overview-head">
                <button class="ocr-toggle" class:on={showOcr} onclick={() => (showOcr = !showOcr)}>
                  🖥️ {currentLang === 'bg' ? 'Текст от видеото' : 'On-screen text'} ({ocrItems.length})
                </button>
                {#if showOcr}
                  <span class="ocr-legend">
                    <i class="lg-speech"></i>{currentLang === 'bg' ? 'Реч' : 'Speech'}
                    <i class="lg-ocr"></i>{currentLang === 'bg' ? 'Текст на екрана' : 'On-screen text'}
                  </span>
                {/if}
              </div>
              {#if showOcr}
                <div class="ocr-track">
                  {#each transcriptDisplaySegments as seg}
                    <span
                      class="ocr-track-speech"
                      style={`left:${(seg.start_ms / timelineDurationMs) * 100}%;width:${Math.max(0.4, ((seg.end_ms - seg.start_ms) / timelineDurationMs) * 100)}%`}
                    ></span>
                  {/each}
                  {#each ocrItems as o}
                    <button
                      class="ocr-track-mark"
                      style={`left:${(o.frame_ms / timelineDurationMs) * 100}%`}
                      onclick={() => seekToMs(o.frame_ms)}
                      title={`${formatTime(o.frame_ms)} · ${o.text}`}
                      aria-label={o.text}
                    ></button>
                  {/each}
                  <span
                    class="ocr-track-playhead"
                    style={`left:${Math.min(100, (currentTimeMs / timelineDurationMs) * 100)}%`}
                  ></span>
                </div>
              {/if}
            </div>
          {/if}

          <div class="transcript-segments" class:editing-active={isEditingTranscript}>
            {#if isEditingTranscript}
              <div class="editing-banner">
                💡 {currentLang === 'bg'
                  ? 'Кликнете върху сегмент за прослушване, редактирайте текста на място в реално време.'
                  : 'Click any segment to hear audio, edit the text in real time.'}
              </div>
            {/if}
            {#each transcriptDisplaySegments as seg}
              {@const cinfo = getSegmentClaimInfo(seg)}
              {#if showOcr && !isEditingTranscript}
                {#each ocrBySegment.map.get(seg.start_ms) ?? [] as o}
                  {@render ocrChip(o)}
                {/each}
              {/if}
              {#if isEditingTranscript}
                <span
                  class="transcript-segment-editable"
                  contenteditable="true"
                  onclick={() => seekToMs(seg.start_ms)}
                  onkeydown={(e) => e.key === 'Enter' && seekToMs(seg.start_ms)}
                  oninput={(e) => handleSegmentEdit(seg, (e.target as HTMLElement).innerText)}
                  role="textbox"
                  tabindex="0"
                  title="Кликни за прослушване, пиши за редакция"
                >{seg.text} </span>
              {:else}
                <span
                  id={`seg-${seg.start_ms}`}
                  class="transcript-segment"
                  class:active={currentTimeMs >= seg.start_ms && currentTimeMs <= seg.end_ms}
                  class:highlighted-pulse={highlightedSegmentMs === seg.start_ms}
                  class:claim-mark-contradicted={cinfo?.verdict === 'contradicted'}
                  class:claim-mark-supported={cinfo?.verdict === 'supported'}
                  class:claim-mark-unverifiable={cinfo?.verdict === 'unverifiable' || cinfo?.verdict === 'already_checked'}
                  onclick={() => {
                    seekToMs(seg.start_ms);
                    if (cinfo) {
                      jumpToClaim(cinfo.claimId);
                    }
                  }}
                  role="button"
                  tabindex="0"
                  onkeydown={(e) => {
                    if (e.key === 'Enter') {
                      seekToMs(seg.start_ms);
                      if (cinfo) jumpToClaim(cinfo.claimId);
                    }
                  }}
                  title={cinfo ? `Твърдение #${cinfo.index}: ${cinfo.verdict}` : `Сегмент: ${Math.round(seg.start_ms / 1000)}s`}
                >
                  {#if cinfo}
                    <span
                      class="claim-badge-inline {cinfo.verdict}"
                      onclick={(e) => {
                        e.stopPropagation();
                        jumpToClaim(cinfo.claimId);
                      }}
                      role="button"
                      tabindex="0"
                      onkeydown={(e) => e.key === 'Enter' && jumpToClaim(cinfo.claimId)}
                      title="Отиди на твърдение #{cinfo.index}"
                    >
                      [Т{cinfo.index}{cinfo.source === 'screen' ? ' 🖥️' : ''}]
                    </span>
                  {/if}
                  {seg.text}{' '}
                </span>
              {/if}
            {/each}
          </div>
          {#if showOcr && !isEditingTranscript && ocrBySegment.tail.length > 0}
            <div class="ocr-tail">
              {#each ocrBySegment.tail as o}
                {@render ocrChip(o)}
              {/each}
            </div>
          {/if}
        {:else}
          <p class="empty-hint">{currentLang === 'bg' ? 'Няма налична стенограма.' : 'No transcript available.'}</p>
        {/if}
      </div>

      <div id="sec-aegis" class="section-card aegis-section">
        <div class="section-header aegis-header">
          <div>
            <div class="aegis-title-row">
              <h3>🛡️ {currentLang === 'bg' ? 'Проверка за манипулирано видео и глас' : 'AEGIS Deepfake & Synthetic Forensics'}</h3>
              {#if detail.aegis_deepfake}
                {@const aegisState = normalizeAegisVerdict(detail.aegis_deepfake.overall_verdict, detail.aegis_deepfake.audio?.synthetic, detail.aegis_deepfake.video?.synthetic)}
                <span class="aegis-verdict-pill aegis-verdict-{aegisState}">
                  {#if aegisState === 'deepfake'}
                    🚨 {currentLang === 'bg' ? 'МАНИПУЛИРАН ЗАПИС' : 'DEEPFAKE DETECTED'}
                  {:else if aegisState === 'suspicious'}
                    ⚠️ {currentLang === 'bg' ? 'СЪМНИТЕЛЕН ЗАПИС' : 'SUSPICIOUS MEDIA'}
                  {:else}
                    ✅ {currentLang === 'bg' ? 'ИСТИНСКИ ЗАПИС' : 'AUTHENTIC MEDIA'}
                  {/if}
                </span>
              {:else}
                <span class="aegis-verdict-pill aegis-verdict-unscanned">
                  ⚪ {currentLang === 'bg' ? 'НЕ Е ПРОВЕРЕНО' : 'NOT SCANNED'}
                </span>
              {/if}
            </div>
            <span class="section-subhint">
              {currentLang === 'bg'
                ? 'Проверка за изкуствен глас, подменено лице и разминаване в говора'
                : 'Voice synthesis detection, visual artifacts, and lip-sync verification'}
            </span>
          </div>
          <button
            class="aegis-scan-btn"
            disabled={isAegisScanning}
            onclick={handleTriggerAegis}
          >
            {#if isAegisScanning}
              <span class="aegis-spinner">⚡</span> {currentLang === 'bg' ? 'Проверка...' : 'Scanning...'}
            {:else if detail.aegis_deepfake}
              🔄 {currentLang === 'bg' ? 'Провери отново' : 'Rescan Media'}
            {:else}
              ⚡ {currentLang === 'bg' ? 'Провери записа' : 'Run Deepfake Scan'}
            {/if}
          </button>
        </div>

        {#if aegisScanError}
          <div class="aegis-error-banner">
            ⚠️ {aegisScanError}
          </div>
        {/if}

        {#if detail.aegis_deepfake}
          <div class="aegis-grid">
            <div class="aegis-card aegis-audio-card" class:synthetic-detected={detail.aegis_deepfake.audio.synthetic}>
              <div class="aegis-card-top">
                <div class="aegis-card-title">
                  <span class="aegis-icon">🎵</span>
                  <div>
                    <h4>{currentLang === 'bg' ? 'Звук и глас' : 'Audio & Voice'}</h4>
                    <span class="aegis-class-label">
                      {#if detail.aegis_deepfake.audio.classification === 'cloned_voice'}
                        🎭 {currentLang === 'bg' ? 'Клониран глас' : 'Cloned AI Voice'}
                      {:else if detail.aegis_deepfake.audio.classification === 'robotic_tts'}
                        🤖 {currentLang === 'bg' ? 'Генериран глас' : 'Robotic Voice (TTS)'}
                      {:else}
                        🎙️ {currentLang === 'bg' ? 'Естествен глас' : 'Authentic Human Voice'}
                      {/if}
                    </span>
                  </div>
                </div>
                <div class="aegis-score-box" class:score-high={detail.aegis_deepfake.audio.score >= 0.7} class:score-med={detail.aegis_deepfake.audio.score >= 0.4 && detail.aegis_deepfake.audio.score < 0.7}>
                  <span class="aegis-score-num">{Math.round(detail.aegis_deepfake.audio.score * 100)}%</span>
                  <span class="aegis-score-label">{currentLang === 'bg' ? 'Изкуствен' : 'Synthetic'}</span>
                </div>
              </div>

              <div class="aegis-meter-track">
                <div class="aegis-meter-fill" style="width: {Math.min(100, Math.round(detail.aegis_deepfake.audio.score * 100))}%" class:fill-alert={detail.aegis_deepfake.audio.score >= 0.7}></div>
              </div>

              <div class="aegis-attribution-box">
                {#if detail.aegis_deepfake.audio.target_person && (detail.aegis_deepfake.audio.target_confidence ?? 0) >= 0.8}
                  <div class="aegis-target-alert">
                    <span class="target-badge-icon">🎯</span>
                    <div class="target-badge-info">
                      <strong>{currentLang === 'bg' ? 'Приписан глас на:' : 'Attributed Voice:'}</strong>
                      <span class="target-name">{detail.aegis_deepfake.audio.target_person}</span>
                      <span class="target-conf">({Math.round((detail.aegis_deepfake.audio.target_confidence ?? 0) * 100)}% {currentLang === 'bg' ? 'точност' : 'confidence'})</span>
                    </div>
                  </div>
                {:else if detail.aegis_deepfake.audio.synthetic}
                  <div class="aegis-target-unattributed">
                    👤 {currentLang === 'bg' ? 'Изкуствен глас без сигурно разпознаване на конкретно лице (< 80%)' : 'Synthetic voice detected without high attribution confidence (< 80%)'}
                  </div>
                {:else}
                  <div class="aegis-target-clean">
                    ✓ {currentLang === 'bg' ? 'Няма данни за подправен глас' : 'No public figure voice cloning detected'}
                  </div>
                {/if}
              </div>

              {#if detail.aegis_deepfake.audio.anomalies.length > 0}
                <div class="aegis-anomalies-list">
                  <span class="anomalies-head">{currentLang === 'bg' ? 'Отклонения:' : 'Anomalies:'}</span>
                  {#each detail.aegis_deepfake.audio.anomalies as anomaly}
                    <span class="aegis-anomaly-chip">⚡ {anomaly}</span>
                  {/each}
                </div>
              {/if}

              <div class="aegis-metrics-grid">
                {#if detail.aegis_deepfake.audio.metrics?.f0_mean_hz !== undefined}
                  <div class="metric-item">
                    <span class="m-label">F0 Pitch:</span>
                    <span class="m-val">{detail.aegis_deepfake.audio.metrics.f0_mean_hz} Hz</span>
                  </div>
                {/if}
                {#if (detail.aegis_deepfake.audio.metrics?.f0_std_hz ?? detail.aegis_deepfake.audio.metrics?.f0_std) !== undefined}
                  <div class="metric-item">
                    <span class="m-label">F0 Std:</span>
                    <span class="m-val">{detail.aegis_deepfake.audio.metrics.f0_std_hz ?? detail.aegis_deepfake.audio.metrics.f0_std} Hz</span>
                  </div>
                {/if}
                {#if (detail.aegis_deepfake.audio.metrics?.spectral_cutoff_khz ?? detail.aegis_deepfake.audio.metrics?.vocoder_cutoff_ratio) !== undefined}
                  <div class="metric-item">
                    <span class="m-label">{detail.aegis_deepfake.audio.metrics?.spectral_cutoff_khz !== undefined ? 'Честотен праг:' : '12kHz Cutoff:'}</span>
                    <span class="m-val">{detail.aegis_deepfake.audio.metrics.spectral_cutoff_khz !== undefined ? `${detail.aegis_deepfake.audio.metrics.spectral_cutoff_khz} kHz` : detail.aegis_deepfake.audio.metrics.vocoder_cutoff_ratio}</span>
                  </div>
                {/if}
              </div>
            </div>

            <div class="aegis-card aegis-video-card" class:synthetic-detected={detail.aegis_deepfake.video.synthetic}>
              <div class="aegis-card-top">
                <div class="aegis-card-title">
                  <span class="aegis-icon">🎬</span>
                  <div>
                    <h4>{currentLang === 'bg' ? 'Картина и лице' : 'Video & Face'}</h4>
                    <span class="aegis-class-label">
                      {#if detail.aegis_deepfake.video.classification === 'face_swap'}
                        🎭 {currentLang === 'bg' ? 'Подменено лице' : 'Face-Swap Deepfake'}
                      {:else if detail.aegis_deepfake.video.classification === 'generative_video'}
                        🤖 {currentLang === 'bg' ? 'Генерирано видео' : 'Generative Video'}
                      {:else}
                        📹 {currentLang === 'bg' ? 'Реално заснето видео' : 'Authentic Camera Capture'}
                      {/if}
                    </span>
                  </div>
                </div>
                <div class="aegis-score-box" class:score-high={detail.aegis_deepfake.video.score >= 0.7} class:score-med={detail.aegis_deepfake.video.score >= 0.4 && detail.aegis_deepfake.video.score < 0.7}>
                  <span class="aegis-score-num">{Math.round(detail.aegis_deepfake.video.score * 100)}%</span>
                  <span class="aegis-score-label">{currentLang === 'bg' ? 'Изкуствено' : 'Synthetic'}</span>
                </div>
              </div>

              <div class="aegis-meter-track">
                <div class="aegis-meter-fill" style="width: {Math.min(100, Math.round(detail.aegis_deepfake.video.score * 100))}%" class:fill-alert={detail.aegis_deepfake.video.score >= 0.7}></div>
              </div>

              <div class="aegis-attribution-box">
                {#if detail.aegis_deepfake.video.target_person && (detail.aegis_deepfake.video.target_confidence ?? 0) >= 0.8}
                  <div class="aegis-target-alert">
                    <span class="target-badge-icon">🎯</span>
                    <div class="target-badge-info">
                      <strong>{currentLang === 'bg' ? 'Приписано лице на:' : 'Attributed Face:'}</strong>
                      <span class="target-name">{detail.aegis_deepfake.video.target_person}</span>
                      <span class="target-conf">({Math.round((detail.aegis_deepfake.video.target_confidence ?? 0) * 100)}% {currentLang === 'bg' ? 'точност' : 'confidence'})</span>
                    </div>
                  </div>
                {:else if detail.aegis_deepfake.video.synthetic}
                  <div class="aegis-target-unattributed">
                    👤 {currentLang === 'bg' ? 'Обработено лице без сигурно разпознаване на конкретна личност (< 80%)' : 'Synthetic face detected without high attribution confidence (< 80%)'}
                  </div>
                {:else}
                  <div class="aegis-target-clean">
                    ✓ {currentLang === 'bg' ? 'Няма данни за подменено лице' : 'No public figure face swapping detected'}
                  </div>
                {/if}
              </div>

              {#if detail.aegis_deepfake.video.anomalies.length > 0}
                <div class="aegis-anomalies-list">
                  <span class="anomalies-head">{currentLang === 'bg' ? 'Отклонения:' : 'Anomalies:'}</span>
                  {#each detail.aegis_deepfake.video.anomalies as anomaly}
                    <span class="aegis-anomaly-chip">⚡ {anomaly}</span>
                  {/each}
                </div>
              {/if}

              <div class="aegis-metrics-grid">
                {#if detail.aegis_deepfake.video.metrics?.seam_gradient_ratio !== undefined}
                  <div class="metric-item">
                    <span class="m-label">Seam Gradient:</span>
                    <span class="m-val">{detail.aegis_deepfake.video.metrics.seam_gradient_ratio}x</span>
                  </div>
                {/if}
                {#if (detail.aegis_deepfake.video.metrics?.mean_fft_spectrum_ratio ?? detail.aegis_deepfake.video.metrics?.fft_ratio) !== undefined}
                  <div class="metric-item">
                    <span class="m-label">FFT спектър:</span>
                    <span class="m-val">{detail.aegis_deepfake.video.metrics.mean_fft_spectrum_ratio ?? detail.aegis_deepfake.video.metrics.fft_ratio}</span>
                  </div>
                {/if}
                {#if (detail.aegis_deepfake.video.metrics?.mean_laplacian_variance ?? detail.aegis_deepfake.video.metrics?.laplacian_var) !== undefined}
                  <div class="metric-item">
                    <span class="m-label">Рязкост:</span>
                    <span class="m-val">{detail.aegis_deepfake.video.metrics.mean_laplacian_variance ?? detail.aegis_deepfake.video.metrics.laplacian_var}</span>
                  </div>
                {/if}
              </div>
            </div>
          </div>

          {#if detail.aegis_deepfake.multimodal}
            <div class="aegis-multimodal-bar" class:av-mismatch={detail.aegis_deepfake.multimodal.av_sync_mismatch}>
              <div class="av-sync-info">
                <span class="av-icon">🔄</span>
                <div>
                  <strong>{currentLang === 'bg' ? 'Синхрон между звук и устни' : 'Lip-Sync Verification'}</strong>
                  <span class="av-desc">
                    {#if detail.aegis_deepfake.multimodal.av_sync_mismatch}
                      {currentLang === 'bg' ? '⚠️ Открито е разминаване между говора и движението на устните' : '⚠️ Phoneme-viseme desynchronization detected'}
                    {:else}
                      {currentLang === 'bg' ? '✓ Нормален синхрон между говор и устни' : '✓ Normal lip-sync temporal coherence'}
                    {/if}
                  </span>
                </div>
              </div>
              {#if detail.aegis_deepfake.multimodal.av_sync_score !== null && detail.aegis_deepfake.multimodal.av_sync_score !== undefined}
                <div class="av-score-pill">
                  {Math.round(detail.aegis_deepfake.multimodal.av_sync_score * 100)}% {currentLang === 'bg' ? 'съвпадение' : 'match'}
                </div>
              {/if}
            </div>
          {/if}

          {#if detail.aegis_deepfake.summary_bg}
            <div class="aegis-summary-box">
              <span class="summary-title">📝 {currentLang === 'bg' ? 'Заключение:' : 'Summary:'}</span>
              <p>{detail.aegis_deepfake.summary_bg}</p>
            </div>
          {/if}
        {:else}
          <div class="aegis-empty-state">
            <p>
              {currentLang === 'bg'
                ? 'Записът още не е проверен за манипулация. Натиснете бутона горе за проверка на звука и картината.'
                : 'This media has not been scanned yet. Click the button above to run audio and video verification.'}
            </p>
          </div>
        {/if}
      </div>
      </div>

      <div class="drawer-col drawer-col-right">
      <div id="sec-claims" class="section-card claims-section">
        <div class="section-header">
          <div>
            <h3>{t.claimsHeading}</h3>
            <span class="section-subhint">
              {currentLang === 'bg'
                ? 'Бърза проверка в български и международни източници'
                : 'Direct search tools and external evidence verification'}
            </span>
          </div>
        </div>

        {#if detail.claims.length === 0}
          <p class="empty-hint">{t.noClaimsFound}</p>
        {:else}
          {#each detail.claims as cwith, cIdx}
            {@const claim = cwith.claim}
            {@const src = claimSource(claim.text_bg)}
            {@const vColor = verdictColorKey(claim.verdict)}
            <div
              id={`claim-${claim.id}`}
              class="claim-block claim-verdict-{vColor}"
              class:claim-pulse={activeClaimId === claim.id}
            >
              <div class="claim-header">
                <span class="claim-index-pill">Т{cIdx + 1}</span>
                <span class="claim-type-tag">{claim.claim_type}</span>
                <span
                  class="claim-source-tag {src}"
                  title={currentLang === 'bg'
                    ? 'Откъде идва твърдението: от речта, от текста на екрана или от двете'
                    : 'Where the claim comes from: speech, on-screen text, or both'}
                >
                  {src === 'screen' ? '🖥️' : src === 'both' ? '🎙️+🖥️' : '🎙️'} {claimSourceLabel(src)}
                </span>
                <span class="verdict-tag {claim.verdict?.toLowerCase()}">
                  {formatVerdictDisplay(claim.verdict, claim.label, currentLang)}
                </span>

                <button
                  class="btn btn-shortcut btn-sm"
                  onclick={() => jumpToSegment(claim.start_ms)}
                  title="Отиди на точния цитат в стенограмата"
                >
                  📍 {currentLang === 'bg' ? 'Към цитата в текста' : 'Jump to transcript'}
                </button>

                <button
                  class="btn btn-text btn-sm"
                  onclick={() => seekToMs(claim.start_ms || 0)}
                  title="Пусни видеото от този момент"
                >
                  ⏱ {Math.round((claim.start_ms || 0) / 1000)}s - {Math.round((claim.end_ms || 0) / 1000)}s
                </button>

                <button
                  class="btn btn-text btn-sm"
                  onclick={() => handleCopyCitation(cwith)}
                  title="Копирай цитат с метаданни за доклад"
                >
                  📋 {currentLang === 'bg' ? 'Цитат' : 'Cite'}
                </button>

                {#if cwith.evidence.length > 0}
                  <button
                    class="btn btn-shortcut btn-sm"
                    onclick={() => {
                      const firstValid = cwith.evidence.find(e => e.stance === 'supports' || e.stance === 'contradicts') || cwith.evidence[0];
                      if (firstValid) jumpToEvidence(firstValid.id);
                    }}
                    title="Отиди на намереното доказателство за това твърдение"
                  >
                    🎯 {currentLang === 'bg' ? 'Към доказателството' : 'To evidence'}
                  </button>
                {/if}
              </div>

              <div class="claim-text">{claim.text_bg}</div>

              {#if claim.attribution && claim.attribution.length > 0}
                <div class="attribution-box">
                  {#each claim.attribution as attr}
                    <div class="attr-row">
                      <span class="attr-label">{t.assertedPart}</span>
                      <span class="attr-val">{attr.asserted_part || attr.actor}</span>
                    </div>
                    <div class="attr-row">
                      <span class="attr-label">{t.attributedPart}</span>
                      <span class="attr-val">{attr.attributed_part || `${attr.act} ${attr.about}`}</span>
                    </div>
                  {/each}
                </div>
              {/if}

              {#if claim.reasoning_bg}
                {@const rb = parseReasoningBreakdown(claim.reasoning_bg, claim.verdict)}
                <div class="claim-reasoning reasoning-verdict-{vColor}">
                  <div class="reasoning-head">
                    <span class="reasoning-title">
                      ⚖️ {currentLang === 'bg' ? 'Мотиви и оценка' : 'Reasoning & Evaluation'}
                    </span>
                    {#if rb.items.length > 0}
                      <span class="reasoning-count-pill">
                        {rb.items.length} {currentLang === 'bg' ? (rb.items.length === 1 ? 'елемент' : 'елемента') : 'checks'}
                      </span>
                    {/if}
                  </div>

                  {#if rb.items.length > 0}
                    <div class="reasoning-statements-list">
                      {#each rb.items as stmtItem, sIdx}
                        <div class="reasoning-stmt-card stmt-{stmtItem.status}">
                          <div class="stmt-top-row">
                            <span class="stmt-index">#{sIdx + 1}</span>
                            <span class="stmt-status-pill {stmtItem.status}">
                              {#if stmtItem.status === 'supported'}
                                ✓ {currentLang === 'bg' ? 'Потвърдено' : 'Supported'}
                              {:else if stmtItem.status === 'contradicted'}
                                ✕ {currentLang === 'bg' ? 'Опровергано' : 'Contradicted'}
                              {:else}
                                ? {currentLang === 'bg' ? 'Непроверимо' : 'Unverifiable'}
                              {/if}
                            </span>
                            {#if stmtItem.statement}
                              <strong class="stmt-subject">{stmtItem.statement}</strong>
                            {/if}
                            {#if stmtItem.passageRefs.length > 0}
                              <span class="stmt-refs">
                                {#each stmtItem.passageRefs as pref}
                                  {@const matchedEv = cwith.evidence.find((e) => (e as any).passage_id?.toLowerCase() === pref)}
                                  {#if matchedEv}
                                    <button
                                      type="button"
                                      class="stmt-ref-btn"
                                      onclick={() => jumpToEvidence(matchedEv.id)}
                                      title={currentLang === 'bg' ? 'Отиди към цитирания източник' : 'Jump to cited source'}
                                    >
                                      [{pref}]
                                    </button>
                                  {:else}
                                    <span class="stmt-ref-tag">[{pref}]</span>
                                  {/if}
                                {/each}
                              </span>
                            {/if}
                          </div>
                          <div class="stmt-detail">{stmtItem.detail}</div>
                          {#if stmtEdit && stmtEdit.key === `${claim.id}-${sIdx}`}
                            <div class="claim-redact-box">
                              <textarea rows="2" bind:value={stmtEdit.text}></textarea>
                              <textarea
                                rows="2"
                                bind:value={stmtEdit.note}
                                placeholder={currentLang === 'bg' ? 'Защо е грешно / какво да се провери (по желание)' : 'Why it is wrong / what to check (optional)'}
                              ></textarea>
                              <div class="redact-actions">
                                <button class="btn-feedback correct" disabled={isRedacting} onclick={() => submitStmtEdit(claim.text_bg)}>
                                  🔁 {currentLang === 'bg' ? 'Запази и провери отново' : 'Save & re-verify'}
                                </button>
                                <button class="btn-feedback" onclick={() => (stmtEdit = null)}>
                                  {currentLang === 'bg' ? 'Отказ' : 'Cancel'}
                                </button>
                              </div>
                            </div>
                          {:else}
                            <button class="btn-feedback" onclick={() => startStmtEdit(claim.id, sIdx, stmtItem.statement || stmtItem.detail, stmtItem.detail)}>
                              ✏️ {currentLang === 'bg' ? 'Редактирай изявлението' : 'Edit statement'}
                            </button>
                          {/if}
                        </div>
                      {/each}
                    </div>
                  {/if}

                  {#if rb.summary}
                    <div class="reasoning-summary-footer summary-{vColor}">
                      <span class="summary-badge">
                        📌 {currentLang === 'bg' ? 'Обобщение:' : 'Summary:'}
                      </span>
                      <span class="summary-text">{rb.summary}</span>
                    </div>
                  {/if}

                  {#if rb.extraNotes.length > 0}
                    <div class="reasoning-extra-notes">
                      {#each rb.extraNotes as note}
                        <div class="reasoning-extra-note">{note}</div>
                      {/each}
                    </div>
                  {/if}
                </div>
              {/if}

              {#if redactingClaimId === claim.id}
                <div class="claim-redact-box">
                  <textarea rows="3" bind:value={redactText} placeholder={currentLang === 'bg' ? 'Коригирано твърдение' : 'Corrected claim'}></textarea>
                  <textarea rows="2" bind:value={redactNotes} placeholder={currentLang === 'bg' ? 'Бележки към опровергаването / обобщението (по желание)' : 'Notes on the refutation / summary (optional)'}></textarea>
                  <div class="redact-actions">
                    <button class="btn-feedback correct" disabled={isRedacting} onclick={submitRedact}>
                      🔁 {currentLang === 'bg' ? 'Запази и провери отново' : 'Save & re-verify'}
                    </button>
                    <button class="btn-feedback" onclick={() => (redactingClaimId = null)}>
                      {currentLang === 'bg' ? 'Отказ' : 'Cancel'}
                    </button>
                  </div>
                </div>
              {:else}
                <button class="btn-feedback" onclick={() => startRedact(claim)}>
                  ✏️ {currentLang === 'bg' ? 'Редактирай и провери отново' : 'Edit & re-verify'}
                </button>
              {/if}

              <div class="claim-feedback-bar">
                <span class="feedback-hint">{currentLang === 'bg' ? 'Оценка на резултата:' : 'Feedback:'}</span>
                <button
                  class="btn-feedback correct"
                  class:selected={feedbackSubmitted[claim.id] === 'correct'}
                  onclick={() => handleSendFeedback(claim.id, 'correct')}
                  title="Преценката на системата е точна"
                >
                  👍 {currentLang === 'bg' ? 'Вярна преценка' : 'Correct'}
                </button>
                <button
                  class="btn-feedback wrong"
                  class:selected={feedbackSubmitted[claim.id] === 'wrong_verdict'}
                  onclick={() => {
                    feedbackPickerClaimId = feedbackPickerClaimId === claim.id ? null : claim.id;
                  }}
                  title="Преценката е погрешна"
                >
                  👎 {currentLang === 'bg' ? 'Грешна преценка' : 'Wrong'}
                </button>
                <button
                  class="btn-feedback unrelated"
                  class:selected={feedbackSubmitted[claim.id] === 'unrelated_evidence'}
                  onclick={() => handleSendFeedback(claim.id, 'unrelated_evidence')}
                  title="Доказателството не се отнася за събитието"
                >
                  ⚠️ {currentLang === 'bg' ? 'Несвързано доказателство' : 'Unrelated'}
                </button>
                {#if feedbackSubmitted[claim.id]}
                  <span class="feedback-saved-badge">✓ {currentLang === 'bg' ? 'Запазено' : 'Saved'}</span>
                {/if}
              </div>

              {#if feedbackPickerClaimId === claim.id}
                <div class="feedback-verdict-picker">
                  <span class="picker-label">{currentLang === 'bg' ? 'Истински резултат:' : 'True verdict:'}</span>
                  <button class="btn btn-xs btn-outline" onclick={() => handleSendFeedback(claim.id, 'wrong_verdict', 'SUPPORTED')}>
                    ✓ {currentLang === 'bg' ? 'Потвърдено' : 'Supported'}
                  </button>
                  <button class="btn btn-xs btn-outline" onclick={() => handleSendFeedback(claim.id, 'wrong_verdict', 'CONTRADICTED')}>
                    ✕ {currentLang === 'bg' ? 'Опровергано' : 'Contradicted'}
                  </button>
                  <button class="btn btn-xs btn-outline" onclick={() => handleSendFeedback(claim.id, 'wrong_verdict', 'UNVERIFIABLE')}>
                    ? {currentLang === 'bg' ? 'Непроверимо' : 'Unverifiable'}
                  </button>
                </div>
              {/if}

              <div class="factcheck-action-bar">
                <span class="factcheck-bar-title">🔍 {currentLang === 'bg' ? 'Провери в:' : 'Check in:'}</span>
                <div class="factcheck-links-grid">
                  <a
                    href={`https://www.google.com/search?q=${encodeURIComponent('site:factcheck.bg OR site:bnr.bg ' + claim.text_bg)}`}
                    target="_blank"
                    rel="noopener noreferrer"
                    class="factcheck-link-btn factcheck-bg"
                    title="Търсене в архивите на Factcheck.bg и БНР Проверено"
                    onclick={(e) => {
                      e.preventDefault();
                      openExternalUrl(`https://www.google.com/search?q=${encodeURIComponent('site:factcheck.bg OR site:bnr.bg ' + claim.text_bg)}`);
                    }}
                  >
                    🇧🇬 Factcheck.bg / БНР ↗
                  </a>
                  <a
                    href={`https://toolbox.google.com/factcheck/explorer/search/list:recent?query=${encodeURIComponent(claim.text_bg)}`}
                    target="_blank"
                    rel="noopener noreferrer"
                    class="factcheck-link-btn factcheck-google"
                    title="Търсене в Google Fact Check Explorer"
                    onclick={(e) => {
                      e.preventDefault();
                      openExternalUrl(`https://toolbox.google.com/factcheck/explorer/search/list:recent?query=${encodeURIComponent(claim.text_bg)}`);
                    }}
                  >
                    🔎 Google Fact Check ↗
                  </a>
                  <a
                    href={`https://www.google.com/search?q=${encodeURIComponent('site:bta.bg ' + claim.text_bg)}`}
                    target="_blank"
                    rel="noopener noreferrer"
                    class="factcheck-link-btn factcheck-bta"
                    title="Търсене в архива на БТА"
                    onclick={(e) => {
                      e.preventDefault();
                      openExternalUrl(`https://www.google.com/search?q=${encodeURIComponent('site:bta.bg ' + claim.text_bg)}`);
                    }}
                  >
                    📰 БТА Архив ↗
                  </a>
                  <a
                    href={`https://www.google.com/search?q=${encodeURIComponent(claim.text_bg)}`}
                    target="_blank"
                    rel="noopener noreferrer"
                    class="factcheck-link-btn factcheck-web"
                    title="Общо търсене на твърдението в Google"
                    onclick={(e) => {
                      e.preventDefault();
                      openExternalUrl(`https://www.google.com/search?q=${encodeURIComponent(claim.text_bg)}`);
                    }}
                  >
                    🌐 Google Търсене ↗
                  </a>
                  <button
                    class="btn btn-primary btn-sm factcheck-verify-btn"
                    onclick={() => handleTriggerClaimVerify(claim.text_bg)}
                    title="Провери твърдението в българските медии и Factcheck.bg"
                  >
                    ⚡ {currentLang === 'bg' ? 'Провери твърдението' : 'Verify Claim'}
                  </button>
                </div>
              </div>

              <div class="evidence-list">
                <h4>{t.evidenceHeading}</h4>
                {#if cwith.evidence.filter(ev => !isJunkSource(ev)).length === 0}
                  <div class="evidence-empty-box">
                    <p class="empty-hint">{t.noEvidenceFound}</p>
                    <button
                      class="btn btn-outline btn-sm"
                      onclick={() => handleTriggerClaimVerify(claim.text_bg)}
                    >
                      ⚡ {currentLang === 'bg' ? 'Търси източници в мрежата' : 'Search web sources'}
                    </button>
                  </div>
                {:else}
                  <div class="evidence-items-grid">
                  {#each cwith.evidence.filter(ev => !isJunkSource(ev)) as ev}
                    <div
                      id={`evidence-${ev.id}`}
                      class="evidence-item"
                      class:expanded={expandedEvidenceId === ev.id}
                      class:evidence-pulse={activeEvidenceId === ev.id}
                    >
                      <div class="evidence-connection-bar">
                        <span class="source-category-pill {sourceCategoryLabel(ev.source_kind).cls}">
                          {sourceCategoryLabel(ev.source_kind).icon} {sourceCategoryLabel(ev.source_kind).label}
                        </span>
                        <span class="stance-badge {stanceClass(ev.stance || '')}">{stanceLabel(ev.stance || '')}</span>
                        <span class="connection-strength-pill">
                          🎯 {ev.stance === 'supports' ? (currentLang === 'bg' ? 'Потвърждава' : 'Supports') : ev.stance === 'contradicts' ? (currentLang === 'bg' ? 'Опровергава' : 'Contradicts') : (currentLang === 'bg' ? 'Свързан контекст' : 'Related Context')}
                        </span>
                      </div>
                      <div class="evidence-meta">
                        <span class="evidence-source">{ev.source_name || ev.source_kind}</span>
                        {#if ev.published_at}
                          <span class="evidence-date">• {ev.published_at.substring(0, 10)}</span>
                        {/if}
                        {#if ev.source_url || (ev as any).url}
                          <button
                            class="btn btn-outline btn-xs preview-act-btn"
                            onclick={() => handleLoadEvidencePreview(ev.source_url || (ev as any).url)}
                            title="Отвори и виж цялата статия"
                          >
                            📖 {currentLang === 'bg' ? 'Преглед на статията' : 'Preview'}
                          </button>
                        {/if}
                      </div>
                      {#if getMatchingKeywords(claim.text_bg, ev.passage || (ev as any).snippet || '').length > 0}
                        <div class="evidence-match-keywords">
                          <span class="keywords-label">🔑 {currentLang === 'bg' ? 'Ключови думи:' : 'Keywords:'}</span>
                          {#each getMatchingKeywords(claim.text_bg, ev.passage || (ev as any).snippet || '') as kw}
                            <span class="keyword-pill">{kw}</span>
                          {/each}
                        </div>
                      {/if}
                      <div class="evidence-snippet">
                        {@html renderHighlightedSnippet(ev.passage || (ev as any).snippet || '', claim.text_bg)}
                      </div>
                      <div class="evidence-actions-row">
                        {#if ev.source_url || (ev as any).url}
                          <a
                            href={ev.source_url || (ev as any).url}
                            target="_blank"
                            rel="noopener noreferrer"
                            class="evidence-link"
                            onclick={(e) => {
                              const u = ev.source_url || (ev as any).url;
                              if (u) {
                                e.preventDefault();
                                openExternalUrl(u);
                              }
                            }}
                          >
                            {(ev as any).title || ev.source_url || (ev as any).url} ↗
                          </a>
                        {/if}
                        <button
                          class="btn btn-text btn-xs toggle-expand-btn"
                          onclick={() => (expandedEvidenceId = expandedEvidenceId === ev.id ? null : ev.id)}
                        >
                          {expandedEvidenceId === ev.id
                            ? (currentLang === 'bg' ? 'Свий ▲' : 'Collapse ▲')
                            : (currentLang === 'bg' ? 'Цял цитат ▼' : 'Full passage ▼')}
                        </button>
                        <button
                          class="btn btn-outline btn-xs delete-evidence-btn"
                          onclick={() => handleDeleteEvidence(claim.id, ev.id)}
                          title={currentLang === 'bg' ? 'Премахни този източник от твърдението' : 'Remove this source from claim'}
                        >
                          🗑️ {currentLang === 'bg' ? 'Премахни' : 'Remove'}
                        </button>
                      </div>
                    </div>
                  {/each}
                  </div>
                {/if}
              </div>
            </div>
          {/each}
        {/if}
      </div>

      {#if detail.people.length > 0}
        <div id="sec-people" class="section-card people-section">
          <div class="section-header">
            <div>
              <h3>{t.peopleHeading}</h3>
              <span class="section-subhint">
                {currentLang === 'bg'
                  ? 'Справка, достоверност и проверени изказвания'
                  : 'Profile, credibility score & verified statements'}
              </span>
            </div>
          </div>

          <div class="people-grid">
            {#each detail.people as person}
              {@const cred = computePersonCredibility(person)}
              <div class="person-card">
                <div class="person-card-header">
                  <div class="person-name-group">
                    <div class="person-title-row">
                      <span class="person-name">{person.canonical_name}</span>
                      <span class="person-entity-badge {getEntityTypeInfo(person.entity_type).cssClass}">
                        {getEntityTypeInfo(person.entity_type).icon} {getEntityTypeInfo(person.entity_type).label}
                      </span>
                    </div>
                    {#if person.role || (person as any).role_hint}
                      <span class="person-role">{person.role || (person as any).role_hint}</span>
                    {/if}
                  </div>
                  <span class="person-cred-badge {cred.badgeClass}">{cred.label}</span>
                </div>

                {#if person.summary}
                  <div class="person-bio-box">
                    <p class="person-bio-text">{person.summary}</p>
                  </div>
                {/if}

                <div class="credibility-meter">
                  <div class="meter-bar-track">
                    <div
                      class="meter-bar-fill {cred.badgeClass}"
                      style={`width: ${cred.score}%`}
                    ></div>
                  </div>
                  <div class="meter-labels">
                    <span>{currentLang === 'bg' ? 'Оценка на достоверност' : 'Credibility score'}</span>
                    <strong>{cred.score}% ({cred.levelLabel})</strong>
                  </div>
                  {#if cred.stats && cred.stats.total > 0}
                    <div class="person-stats-mini">
                      <span>{currentLang === 'bg' ? 'Свързани твърдения:' : 'Claims:'} <strong>{cred.stats.total}</strong></span>
                      {#if cred.stats.contradicted > 0}
                        <span class="stat-warn">• {currentLang === 'bg' ? 'Опровергани:' : 'False:'} <strong>{cred.stats.contradicted}</strong></span>
                      {/if}
                      {#if cred.stats.supported > 0}
                        <span class="stat-ok">• {currentLang === 'bg' ? 'Потвърдени:' : 'Supported:'} <strong>{cred.stats.supported}</strong></span>
                      {/if}
                    </div>
                  {/if}
                </div>

                <div class="person-actions-row">
                  <button
                    class="btn btn-secondary btn-sm"
                    onclick={() => (viewingPersonDossier = person)}
                    title="Отвори аналитичната справка"
                  >
                    📖 {currentLang === 'bg' ? 'Справка' : 'Report'}
                  </button>
                  <button
                    class="btn btn-primary btn-sm person-verify-btn"
                    onclick={() => handleResearchPerson(person)}
                    disabled={isJobActive('person-research', person.id)}
                    title="Проучи лицето от публични регистри и архиви"
                  >
                    {isJobActive('person-research', person.id) ? '...' : '⚡ ' + (currentLang === 'bg' ? 'Проучи' : 'Research')}
                  </button>
                  <button
                    class="btn btn-secondary btn-sm"
                    onclick={() => handleOpenDisambiguate(person)}
                    title="Смени или коригирай това лице (напр. политик вместо футболист)"
                  >
                    ✏️ {currentLang === 'bg' ? 'Коригирай' : 'Edit'}
                  </button>
                  <button
                    class="btn btn-danger btn-sm"
                    onclick={() => handleRemovePersonFromVideo(person)}
                    title="Премахни от това видео"
                  >
                    ✕
                  </button>
                  <a
                    href={`https://www.google.com/search?q=${encodeURIComponent('site:factcheck.bg OR site:bnr.bg "' + person.canonical_name + '"')}`}
                    target="_blank"
                    rel="noopener noreferrer"
                    class="person-link-btn"
                    title="Проверка на изказвания на това лице във Factcheck.bg / БНР"
                    onclick={(e) => {
                      e.preventDefault();
                      openExternalUrl(`https://www.google.com/search?q=${encodeURIComponent('site:factcheck.bg OR site:bnr.bg "' + person.canonical_name + '"')}`);
                    }}
                  >
                    🇧🇬 Factcheck / БНР ↗
                  </a>
                  <a
                    href={`https://bg.wikipedia.org/w/index.php?search=${encodeURIComponent(person.canonical_name)}`}
                    target="_blank"
                    rel="noopener noreferrer"
                    class="person-link-btn"
                    title="Търсене в Уикипедия на български"
                    onclick={(e) => {
                      e.preventDefault();
                      openExternalUrl(`https://bg.wikipedia.org/w/index.php?search=${encodeURIComponent(person.canonical_name)}`);
                    }}
                  >
                    🌐 Уикипедия ↗
                  </a>
                  <a
                    href={`https://news.google.com/search?q=${encodeURIComponent(person.canonical_name)}`}
                    target="_blank"
                    rel="noopener noreferrer"
                    class="person-link-btn"
                    title="Новини за личността в Google Новини"
                    onclick={(e) => {
                      e.preventDefault();
                      openExternalUrl(`https://news.google.com/search?q=${encodeURIComponent(person.canonical_name)}`);
                    }}
                  >
                    📰 Новини ↗
                  </a>
                  {#if person.wikidata_id}
                    <a
                      href={`https://www.wikidata.org/wiki/${person.wikidata_id}`}
                      target="_blank"
                      rel="noopener noreferrer"
                      class="person-link-btn wikidata-link"
                      title="Wikidata ID"
                      onclick={(e) => {
                        e.preventDefault();
                        openExternalUrl(`https://www.wikidata.org/wiki/${person.wikidata_id}`);
                      }}
                    >
                      Wikidata ({person.wikidata_id}) ↗
                    </a>
                  {/if}
                </div>
              </div>
            {/each}
          </div>
        </div>
      {/if}

      {#if detail.creator || detail.video.creator_handle}
        {@const creator = detail.creator || { id: (detail.video.creator_handle || '').toLowerCase(), handle: detail.video.creator_handle || '', display_name: detail.video.creator_display_name, follower_count: detail.video.creator_follower_count ?? null, region: 'BG', first_seen_at: new Date().toISOString(), flagged_count: 0, watchlist: false, expansion_depth: 0 }}
        {@const authorRisk = computeAuthorRisk(creator)}
        <div id="sec-creator" class="section-card creator-section">
          <div class="section-header">
            <div>
              <h3>{t.creatorHeading}</h3>
              <span class="section-subhint">
                {currentLang === 'bg'
                  ? 'Профил на автора, рисков профил и предишни засечени видеа'
                  : 'Author profile, risk tier and previous high-risk flagged videos'}
              </span>
            </div>
            <div class="author-risk-header-badges">
              <span class="author-risk-badge {authorRisk.badgeClass}">{authorRisk.label}</span>
              <span class="author-confidence-pill {authorRisk.confidenceClass}" title={authorRisk.confidenceReason}>
                📊 {currentLang === 'bg' ? 'Увереност: ' : 'Confidence: '}<strong>{authorRisk.confidencePercent}%</strong> ({authorRisk.confidenceText})
              </span>
            </div>
          </div>

          <div class="creator-box">
            <div class="creator-main-row">
              <div>
                <a
                  href={`https://www.tiktok.com/@${creator.handle}`}
                  target="_blank"
                  rel="noopener noreferrer"
                  class="creator-name-link"
                  title="Отвори профила в TikTok"
                  onclick={(e) => {
                    e.preventDefault();
                    openExternalUrl(`https://www.tiktok.com/@${creator.handle}`);
                  }}
                >
                  @{creator.handle} ↗
                </a>
                {#if creator.display_name}
                  <span class="creator-sub">({creator.display_name})</span>
                {/if}
              </div>

              <button
                class="btn btn-sm"
                class:btn-secondary={creator.watchlist}
                class:btn-outline={!creator.watchlist}
                onclick={handleToggleWatchlist}
                disabled={isTogglingWatchlist}
              >
                {creator.watchlist
                  ? (currentLang === 'bg' ? '👁 В наблюдение' : '👁 Watchlisted')
                  : (currentLang === 'bg' ? '+ Добави в наблюдение' : '+ Add to watchlist')}
              </button>
              <a
                href={`https://www.google.com/search?q=${encodeURIComponent('site:tiktok.com/@' + creator.handle + ' OR "' + (creator.display_name || creator.handle) + '"')}`}
                target="_blank"
                rel="noopener noreferrer"
                class="btn btn-outline btn-sm"
                title="Търсене за този автор в Google"
                onclick={(e) => {
                  e.preventDefault();
                  openExternalUrl(`https://www.google.com/search?q=${encodeURIComponent('site:tiktok.com/@' + creator.handle + ' OR "' + (creator.display_name || creator.handle) + '"')}`);
                }}
              >
                🔍 Търси в Google
              </a>
              <button
                class="btn btn-outline btn-sm btn-web-risk"
                onclick={handleAssessAuthorWebRisk}
                disabled={isAssessingRisk}
                title={currentLang === 'bg' ? 'Проверка за предишни опровержения и надеждност на автора' : 'Scan fact-check archives for this creator'}
              >
                {#if isAssessingRisk}
                  <span class="loading-mini-spin">...</span> {currentLang === 'bg' ? 'Проверка...' : 'Checking...'}
                {:else}
                  🌐 {currentLang === 'bg' ? 'Провери автора' : 'Check author'}
                {/if}
              </button>
            </div>

            <div class="creator-stat-row">
              <span>{t.creatorFlaggedCount} <strong>{creator.flagged_count ?? 0}</strong></span>
              {#if creator.follower_count}
                <span>• Последователи: <strong>{(creator.follower_count).toLocaleString()}</strong></span>
              {/if}
              <span>• Регион: <strong>{creator.region || 'BG'}</strong></span>
            </div>

            <div class="author-risk-assessment-card">
              <div class="risk-assessment-top">
                <span class="risk-assessment-title">
                  🛡️ {currentLang === 'bg' ? 'Надеждност на автора' : 'Author Reliability'}
                </span>
                {#if authorRisk.lastCalculatedAt}
                  <span class="risk-calc-time">
                    🕒 {new Date(authorRisk.lastCalculatedAt).toLocaleTimeString()}
                  </span>
                {/if}
              </div>

              <p class="risk-rationale-text">{authorRisk.rationale}</p>

              <div class="confidence-bar-container" title={authorRisk.confidenceReason}>
                <div class="confidence-bar-label">
                  <span>{currentLang === 'bg' ? 'Точност на оценката:' : 'Assessment confidence:'}</span>
                  <strong>{authorRisk.confidencePercent}% ({authorRisk.confidenceText})</strong>
                </div>
                <div class="confidence-bar-track">
                  <div class="confidence-bar-fill {authorRisk.confidenceClass}" style="width: {authorRisk.confidencePercent}%"></div>
                </div>
                <span class="confidence-reason-hint">ℹ️ {authorRisk.confidenceReason}</span>
              </div>

              {#if authorRisk.webSources && authorRisk.webSources.length > 0}
                <div class="author-web-sources-section">
                  <span class="web-sources-heading">
                    🌐 {currentLang === 'bg' ? 'Намерени публикации (' + authorRisk.webSources.length + '):' : 'Sources found (' + authorRisk.webSources.length + '):'}
                  </span>
                  <div class="web-sources-chips">
                    {#each authorRisk.webSources as src}
                      <a
                        href={src.url}
                        target="_blank"
                        rel="noopener noreferrer"
                        class="web-source-chip"
                        onclick={(e) => {
                          e.preventDefault();
                          openExternalUrl(src.url);
                        }}
                      >
                        <span class="source-domain-tag">{src.domain}</span>
                        <span class="source-title-text">{src.title}</span> ↗
                      </a>
                    {/each}
                  </div>
                </div>
              {/if}
            </div>
          </div>

          <div class="author-other-videos">
            <div class="author-other-header">
              <h4>🎬 {currentLang === 'bg' ? 'Други видеа от този автор в базата:' : 'Other videos by this author:'}</h4>
              {#if isLoadingCreatorVideos}
                <span class="loading-mini-spin">...</span>
              {/if}
            </div>

            {#if creatorVideos.length > 0}
              <div class="other-videos-grid">
                {#each creatorVideos as ov}
                  <div
                    class="other-video-card"
                    onclick={() => selectOtherVideo(ov.id)}
                    role="button"
                    tabindex="0"
                    onkeydown={(e) => e.key === 'Enter' && selectOtherVideo(ov.id)}
                  >
                    <div class="other-vid-top">
                      <span class="other-vid-id">ID: {ov.id}</span>
                      <span class="verdict-tag {ov.top_claim_verdict?.toLowerCase()}">
                        {formatVerdictDisplay(ov.top_claim_verdict, ov.top_claim_label, currentLang)}
                      </span>
                    </div>
                    <div class="other-vid-claim">
                      {ov.top_claim_text || 'Няма извлечено заглавие'}
                    </div>
                    <div class="other-vid-stats">
                      <span>👁 {(ov.views || 0).toLocaleString()}</span>
                      {#if ov.duration_s}
                        <span>⏱ {Math.round(ov.duration_s)}s</span>
                      {/if}
                      <span>Ранг: {Math.round((ov.lead_score || 0) * 100)}%</span>
                      <span class="inspect-btn">{currentLang === 'bg' ? 'Преглед ↗' : 'View ↗'}</span>
                    </div>
                  </div>
                {/each}
              </div>
            {:else if !isLoadingCreatorVideos}
              <div class="creator-empty-box">
                <p class="empty-hint">
                  {currentLang === 'bg'
                    ? 'Няма други записани видеа от този автор в локалната база.'
                    : 'No other videos by this creator in local database.'}
                </p>
                <div class="creator-empty-actions">
                  <a
                    href={`https://www.tiktok.com/@${creator.handle}`}
                    target="_blank"
                    rel="noopener noreferrer"
                    class="btn btn-outline btn-sm"
                    onclick={(e) => {
                      e.preventDefault();
                      openExternalUrl(`https://www.tiktok.com/@${creator.handle}`);
                    }}
                  >
                    🎬 Отвори видеата на @{creator.handle} в TikTok ↗
                  </a>
                </div>
              </div>
            {/if}
          </div>
        </div>
      {/if}

      <div id="sec-metadata" class="section-card metadata-section">
        <button class="collapsible-header" onclick={() => (metadataOpen = !metadataOpen)}>
          <span>{t.metadataHeading}</span>
          <span>{metadataOpen ? '▲' : '▼'}</span>
        </button>
        {#if metadataOpen}
          <pre class="metadata-dump">{JSON.stringify(detail.video, null, 2)}</pre>
        {/if}
      </div>
      </div>
    </div>
  {/if}

  {#if previewModalData}
    <div class="modal-backdrop" onclick={() => (previewModalData = null)} role="presentation">
      <div class="preview-modal" onclick={(e) => e.stopPropagation()} onkeydown={(e) => e.stopPropagation()} role="dialog" aria-modal="true" tabindex="-1">
        <div class="preview-modal-header">
          <div>
            <span class="preview-domain-pill">{previewModalData.domain || 'Уеб източник'}</span>
            <h3 class="preview-modal-title">{previewModalData.title || previewModalData.url}</h3>
          </div>
          <button class="close-btn" onclick={() => (previewModalData = null)}>✕</button>
        </div>
        {#if previewTocItems.length > 0}
          <div class="preview-toc-wrap">
            <BookReaderToc
              variant="compact"
              items={previewTocItems}
              scrollContainer={previewBodyEl}
              title={currentLang === 'bg' ? 'Съдържание' : 'Contents'}
            />
          </div>
        {/if}
        <div class="preview-modal-body" bind:this={previewBodyEl}>
          {#if previewParagraphs.length > 0}
            {#each previewParagraphs as para, pIdx}
              <p id={`preview-para-${pIdx}`} class="preview-paragraph">{@html highlightPlainTextToHtml(para, true)}</p>
            {/each}
          {:else}
            <p class="empty-hint">Няма извлечен текст от този източник.</p>
          {/if}
        </div>
        <div class="preview-modal-footer">
          <a
            href={previewModalData.url}
            target="_blank"
            rel="noopener noreferrer"
            class="btn btn-primary btn-sm"
            onclick={(e) => {
              if (previewModalData?.url) {
                e.preventDefault();
                openExternalUrl(previewModalData.url);
              }
            }}
          >
            Отвори оригиналната страница ↗
          </a>
          <button class="btn btn-secondary btn-sm" onclick={() => (previewModalData = null)}>
            Затвори
          </button>
        </div>
      </div>
    </div>
  {/if}

  {#if googleAiModalData}
    <div class="modal-backdrop" onclick={() => (googleAiModalData = null)} role="presentation">
      <div class="preview-modal google-ai-modal" onclick={(e) => e.stopPropagation()} onkeydown={(e) => e.stopPropagation()} role="dialog" aria-modal="true" tabindex="-1">
        <div class="preview-modal-header">
          <div>
            <span class="preview-domain-pill">{googleAiModalData.source === 'google_ai_mode' ? 'Google AI Mode' : 'Обобщено проучване'}</span>
            <h3 class="preview-modal-title">{googleAiModalData.mode === 'refine' ? 'Поправен текст' : googleAiModalData.query}</h3>
          </div>
          <button class="close-btn" onclick={() => (googleAiModalData = null)}>✕</button>
        </div>
        <div class="preview-modal-body">
          {#if googleAiModalData.text}
            {#each googleAiModalData.text.split('\n\n') as para}
              {#if para.trim()}
                <p class="preview-paragraph">{@html highlightPlainTextToHtml(para.trim(), true)}</p>
              {/if}
            {/each}
          {:else}
            <p class="empty-hint">Няма извлечен отговор.</p>
          {/if}

          {#if googleAiModalData.citations && googleAiModalData.citations.length > 0}
            <div class="google-ai-citations-box">
              <h4 class="citations-title">Намерени първични източници и цитати ({googleAiModalData.citations.length})</h4>
              <ul class="citations-list">
                {#each googleAiModalData.citations as c}
                  <li class="citation-entry">
                    <a
                      href={c.url}
                      target="_blank"
                      rel="noopener noreferrer"
                      class="citation-link"
                      onclick={(e) => {
                        if (c.url) {
                          e.preventDefault();
                          openExternalUrl(c.url);
                        }
                      }}
                    >
                      ↗ {c.title || c.url}
                    </a>
                  </li>
                {/each}
              </ul>
            </div>
          {/if}
        </div>
        <div class="preview-modal-footer">
          {#if googleAiModalData.mode === 'refine'}
            <button
              class="btn btn-primary btn-sm"
              onclick={() => handleApplyGoogleAiRefinedText(googleAiModalData?.text || '')}
            >
              💾 Приложи в стенограмата
            </button>
          {/if}
          <button
            class="btn btn-secondary btn-sm"
            onclick={() => {
              if (googleAiModalData?.text) {
                navigator.clipboard.writeText(googleAiModalData.text);
                copyToast = '✓ Копирано в клипборда!';
                setTimeout(() => (copyToast = null), 2500);
              }
            }}
          >
            📋 Копирай
          </button>
          <button class="btn btn-secondary btn-sm" onclick={() => (googleAiModalData = null)}>
            Затвори
          </button>
        </div>
      </div>
    </div>
  {/if}

  {#if isVideoOutOfView && detail}
    <div class="floating-audio-widget">
      <button class="float-btn float-play" onclick={togglePlayPause} title={isPlaying ? 'Пауза' : 'Възпроизвеждане'}>
        {isPlaying ? '⏸' : '▶'}
      </button>
      <div class="float-controls-group">
        <button class="float-btn float-seek" onclick={() => seekRelative(-5)} title="Назад 5s">
          ⏪
        </button>
        <button class="float-btn float-seek" onclick={() => seekRelative(5)} title="Напред 5s">
          ⏩
        </button>
        <span class="float-time-text">{formatTime(currentTimeMs)}</span>
      </div>
    </div>
  {/if}

  {#if disambiguatingPerson}
    <div class="modal-backdrop" onclick={() => (disambiguatingPerson = null)} role="presentation">
      <div class="modal-dialog" onclick={(e) => e.stopPropagation()} onkeydown={(e) => e.stopPropagation()} role="dialog" tabindex="-1">
        <div class="modal-head">
          <h3>✏️ {currentLang === 'bg' ? 'Смяна на лице:' : 'Disambiguate:'} {disambiguatingPerson.canonical_name}</h3>
          <button class="close-btn" onclick={() => (disambiguatingPerson = null)}>✕</button>
        </div>
        <div class="modal-body">
          <p class="modal-hint">
            {currentLang === 'bg'
              ? 'Ако автоматичното разпознаване е асоциирало видеото с грешно лице (напр. футболист вместо политик), изберете правилното лице от базата или въведете ново.'
              : 'Reassign this detection to another person in the database or specify a new profile.'}
          </p>
          <label>
            <span>{currentLang === 'bg' ? 'Избери съществуваща личност от базата:' : 'Select existing person:'}</span>
            <select bind:value={disambiguateTargetPersonId}>
              <option value={null}>-- {currentLang === 'bg' ? 'Или въведи ново лице отдолу' : 'Or enter new details below'} --</option>
              {#each availablePeople as p}
                <option value={p.id}>{p.canonical_name} ({p.role || p.category || 'Личност'})</option>
              {/each}
            </select>
          </label>
          <hr class="modal-divider" />
          <label>
            <span>{currentLang === 'bg' ? 'Тип субект:' : 'Entity Type:'}</span>
            <select bind:value={disambiguateEntityType}>
              <option value="person">👤 {currentLang === 'bg' ? 'Личност (човек)' : 'Person (individual)'}</option>
              <option value="institution">🏛️ {currentLang === 'bg' ? 'Институция (орган, комисия, съд)' : 'Institution'}</option>
              <option value="channel">📺 {currentLang === 'bg' ? 'Канал / Медия (предаване, YouTube, медия)' : 'Channel / Media'}</option>
              <option value="country">🌍 {currentLang === 'bg' ? 'Държава (геополитически субект)' : 'Country'}</option>
              <option value="group">👥 {currentLang === 'bg' ? 'Група / Партия (партия, коалиция, движение)' : 'Group / Party'}</option>
              <option value="event">📅 {currentLang === 'bg' ? 'Събитие (избори, инцидент, протест)' : 'Event'}</option>
              <option value="infrastructure">🛣️ {currentLang === 'bg' ? 'Инфраструктура (магистрала, мост, язовир, АЕЦ)' : 'Infrastructure'}</option>
              <option value="building">🏢 {currentLang === 'bg' ? 'Сграда / Обект (болница, училище, комплекс)' : 'Building / Facility'}</option>
            </select>
          </label>
          <label>
            <span>{currentLang === 'bg' ? 'Или въведи ново правилно име:' : 'Or enter new name:'}</span>
            <input type="text" bind:value={disambiguateNewName} placeholder="напр. Костадин Костадинов (политик)" />
          </label>
          <label>
            <span>{currentLang === 'bg' ? 'Длъжност / Категория:' : 'Role / Category:'}</span>
            <input type="text" bind:value={disambiguateRole} placeholder="Политик, лидер на партия" />
          </label>
          <label>
            <span>{currentLang === 'bg' ? 'Тагове:' : 'Tags:'}</span>
            <input type="text" bind:value={disambiguateTags} placeholder="[политик], [партия X], [депутат]" />
          </label>
        </div>
        <div class="modal-foot">
          <button class="btn btn-secondary btn-sm" onclick={() => (disambiguatingPerson = null)}>
            {currentLang === 'bg' ? 'Отказ' : 'Cancel'}
          </button>
          <button class="btn btn-primary btn-sm" onclick={handleConfirmDisambiguation}>
            {currentLang === 'bg' ? '✓ Потвърди корекцията' : '✓ Confirm'}
          </button>
        </div>
      </div>
    </div>
  {/if}

  {#if viewingPersonDossier}
    <PersonDossierModal
      person={viewingPersonDossier}
      {currentLang}
      onClose={() => (viewingPersonDossier = null)}
      onResearchTrigger={async (p) => {
        await handleResearchPerson(p);
        const updated = detail?.people?.find((item) => item.id === p.id);
        if (updated) viewingPersonDossier = updated;
      }}
    />
  {/if}
</aside>

<style>
  .detail-drawer {
    width: 620px;
    height: 100%;
    background: var(--surface-bg);
    border-left: 1px solid var(--border-color);
    display: flex;
    flex-direction: column;
    box-shadow: -4px 0 20px rgba(0, 0, 0, 0.12);
    position: relative;
    z-index: 50;
    flex-shrink: 0;
  }

  .detail-drawer.widescreen-mode {
    width: var(--drawer-docked-width, clamp(860px, 68vw, 2400px));
  }

  .detail-drawer.ultrawide-mode {
    width: var(--drawer-docked-width, clamp(1180px, 72vw, 2800px));
  }

  .detail-drawer.eighteen-inch-mode .drawer-header {
    padding: 0.62rem 1.05rem;
  }

  .detail-drawer.eighteen-inch-mode .drawer-toc-bar {
    padding: 0.36rem 0.95rem;
  }

  .drawer-header {
    display: flex;
    align-items: center;
    justify-content: space-between;
    padding: 0.85rem 1.15rem;
    border-bottom: 1px solid var(--border-color);
  }

  .drawer-toc-bar {
    padding: 0.45rem 1rem;
    border-bottom: 1px solid var(--border-color);
    background: var(--surface-bg);
    flex-shrink: 0;
  }

  .preview-toc-wrap {
    padding: 0.45rem 1.1rem;
    border-bottom: 1px solid var(--border-color);
  }

  .drawer-title-group {
    display: flex;
    align-items: center;
    gap: 0.5rem;
  }

  .drawer-title {
    margin: 0;
    font-size: 1.05rem;
    font-weight: 700;
    color: var(--text-heading);
  }

  .video-ref-id {
    font-size: 0.72rem;
    color: var(--text-muted);
  }

  .active-jobs-badge {
    font-size: 0.72rem;
    font-weight: 700;
    color: var(--primary-accent);
    background: var(--primary-accent-alpha);
    border: 1px solid var(--primary-accent);
    padding: 2px 7px;
    border-radius: 999px;
    animation: activePulse 2s infinite ease-in-out;
  }

  @keyframes activePulse {
    0%, 100% { opacity: 1; }
    50% { opacity: 0.65; }
  }

  .close-btn {
    background: none;
    border: none;
    font-size: 1.1rem;
    cursor: pointer;
    color: var(--text-muted);
    padding: 0.25rem 0.5rem;
    border-radius: 4px;
  }

  .close-btn:hover {
    color: var(--text-heading);
    background: var(--hover-bg);
  }

  .drawer-body {
    flex: 1;
    overflow-y: auto;
    padding: 1rem 1.15rem;
    display: flex;
    flex-direction: column;
    gap: 1rem;
    min-height: 0;
  }

  .drawer-col {
    display: contents;
  }

  .detail-drawer.dual-pane-mode .drawer-body {
    display: grid;
    grid-template-columns: var(--drawer-left-col, minmax(360px, 37%)) minmax(0, 1fr);
    gap: 0;
    padding: 0;
    overflow: hidden;
  }

  .detail-drawer.ultrawide-mode .drawer-body {
    grid-template-columns: var(--drawer-left-col, minmax(440px, 36%)) minmax(0, 1fr);
  }

  .detail-drawer.dual-pane-mode .drawer-col {
    display: flex;
    flex-direction: column;
    gap: 0.85rem;
    height: 100%;
    overflow-y: auto;
    padding: 0.88rem 1rem;
    min-height: 0;
  }

  .detail-drawer.eighteen-inch-mode .drawer-col {
    gap: 0.78rem;
    padding: 0.82rem 0.98rem;
  }

  .detail-drawer.dual-pane-mode .drawer-col-left {
    border-right: 1px solid var(--border-color);
  }

  .loading-state {
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

  @keyframes spin {
    to {
      transform: rotate(360deg);
    }
  }

  .media-section {
    width: 100%;
    background: #000;
    border-radius: 8px;
    overflow: hidden;
  }

  .media-section.feed-mode {
    width: 100%;
    background: #000;
    border-radius: 8px;
    overflow: hidden;
    display: flex;
    flex-direction: column;
    border: 1px solid var(--border-color);
    margin-top: 0.75rem;
    margin-bottom: 0.85rem;
  }

  .feed-top-meta {
    display: flex;
    align-items: center;
    justify-content: space-between;
    padding: 0.45rem 0.85rem;
    background: rgba(18, 18, 24, 0.9);
    border-bottom: 1px solid var(--border-color);
  }

  .feed-creator-badge {
    display: flex;
    align-items: center;
    gap: 0.4rem;
  }

  .creator-name {
    font-size: 0.85rem;
    font-weight: 700;
    color: var(--primary-accent);
  }

  .creator-views {
    font-size: 0.75rem;
    color: var(--text-muted);
  }

  .feed-duration-pill {
    font-size: 0.72rem;
    font-weight: 700;
    color: #fff;
    background: rgba(0, 0, 0, 0.6);
    padding: 1px 6px;
    border-radius: 4px;
  }

  .feed-video-container {
    width: 100%;
    height: var(--video-stage-height, 420px);
    background: #000;
    display: flex;
    align-items: center;
    justify-content: center;
  }

  .feed-vertical-player {
    max-width: 100%;
    max-height: 100%;
    height: 100%;
    aspect-ratio: 9 / 16;
    object-fit: contain;
    display: block;
    background: #000;
  }

  .btn-ai-fix {
    background: var(--primary-accent);
    color: #fff;
    font-weight: 600;
  }

  .btn-google-audio {
    background: rgba(37, 99, 235, 0.15);
    color: #3b82f6;
    border: 1px solid rgba(37, 99, 235, 0.35);
    font-weight: 600;
  }

  .btn-google-audio:hover {
    background: rgba(37, 99, 235, 0.25);
  }

  .editing-banner {
    font-size: 0.75rem;
    font-weight: 600;
    color: var(--primary-accent);
    background: var(--primary-accent-alpha);
    border: 1px dashed var(--primary-accent);
    padding: 0.35rem 0.65rem;
    border-radius: 6px;
    margin-bottom: 0.5rem;
    width: 100%;
  }

  .transcript-segment-editable {
    display: inline;
    padding: 1px 3px;
    border-radius: 3px;
    cursor: text;
    background: rgba(59, 130, 246, 0.08);
    border-bottom: 1px dashed #3b82f6;
    outline: none;
  }

  .transcript-segment-editable:focus {
    background: rgba(59, 130, 246, 0.2);
    border-bottom: 1px solid #3b82f6;
  }

  .floating-audio-widget {
    position: absolute;
    bottom: 1.5rem;
    right: 1.5rem;
    display: flex;
    align-items: center;
    gap: 0.4rem;
    background: var(--surface-bg);
    border: 1px solid var(--primary-accent);
    padding: 0.35rem 0.65rem;
    border-radius: 999px;
    box-shadow: 0 4px 16px rgba(0, 0, 0, 0.35);
    z-index: 60;
  }

  .float-btn {
    border: none;
    border-radius: 50%;
    display: flex;
    align-items: center;
    justify-content: center;
    cursor: pointer;
    transition: transform 0.1s ease;
  }

  .float-btn:hover {
    transform: scale(1.08);
  }

  .float-play {
    width: 36px;
    height: 36px;
    background: var(--primary-accent);
    color: #fff;
    font-size: 1rem;
  }

  .float-seek {
    width: 28px;
    height: 28px;
    background: var(--card-bg);
    border: 1px solid var(--border-color);
    color: var(--text-heading);
    font-size: 0.75rem;
  }

  .float-controls-group {
    display: flex;
    align-items: center;
    gap: 0.3rem;
  }

  .float-time-text {
    font-size: 0.75rem;
    font-weight: 700;
    color: var(--text-heading);
    margin-left: 0.25rem;
  }

  .modal-hint {
    margin: 0;
    font-size: 0.8rem;
    color: var(--text-muted);
    line-height: 1.4;
  }

  .modal-divider {
    border: none;
    border-top: 1px solid var(--border-color);
    margin: 0.4rem 0;
  }

  .video-player {
    width: 100%;
    max-height: 280px;
    display: block;
  }

  .section-card {
    background: var(--card-bg);
    border: 1px solid var(--border-color);
    border-radius: 8px;
    padding: 0.85rem;
  }

  .section-card h3 {
    margin: 0;
    font-size: 0.95rem;
    font-weight: 700;
    color: var(--text-heading);
  }

  .section-subhint {
    display: block;
    font-size: 0.72rem;
    color: var(--text-muted);
    margin-top: 0.2rem;
  }

  .section-header {
    display: flex;
    align-items: flex-start;
    justify-content: space-between;
    margin-bottom: 0.65rem;
    gap: 0.5rem;
  }

  .aegis-section {
    border-color: rgba(99, 102, 241, 0.35);
    background: linear-gradient(180deg, rgba(30, 27, 75, 0.15) 0%, var(--card-bg) 100%);
  }

  .aegis-title-row {
    display: flex;
    align-items: center;
    gap: 0.6rem;
    flex-wrap: wrap;
  }

  .aegis-verdict-pill {
    font-size: 0.68rem;
    font-weight: 800;
    padding: 2px 7px;
    border-radius: 999px;
    text-transform: uppercase;
    letter-spacing: 0.03em;
  }

  .aegis-verdict-deepfake {
    background: rgba(239, 68, 68, 0.2);
    color: #ef4444;
    border: 1px solid rgba(239, 68, 68, 0.5);
    animation: badgePulse 1.8s infinite;
  }

  .aegis-verdict-suspicious {
    background: rgba(245, 158, 11, 0.2);
    color: #f59e0b;
    border: 1px solid rgba(245, 158, 11, 0.5);
  }

  .aegis-verdict-authentic {
    background: rgba(16, 185, 129, 0.2);
    color: #10b981;
    border: 1px solid rgba(16, 185, 129, 0.5);
  }

  .aegis-verdict-unscanned {
    background: rgba(148, 163, 184, 0.15);
    color: #94a3b8;
    border: 1px solid rgba(148, 163, 184, 0.3);
  }

  .aegis-scan-btn {
    display: inline-flex;
    align-items: center;
    gap: 0.35rem;
    padding: 0.35rem 0.75rem;
    font-size: 0.75rem;
    font-weight: 700;
    border-radius: 6px;
    background: #4f46e5;
    color: #fff;
    border: 1px solid #6366f1;
    cursor: pointer;
    white-space: nowrap;
    transition: all 0.2s ease;
  }

  .aegis-scan-btn:hover:not(:disabled) {
    background: #4338ca;
    box-shadow: 0 0 10px rgba(99, 102, 241, 0.4);
  }

  .aegis-scan-btn:disabled {
    opacity: 0.6;
    cursor: not-allowed;
  }

  .aegis-spinner {
    display: inline-block;
    animation: cardSpin 1s linear infinite;
  }

  .aegis-grid {
    display: grid;
    grid-template-columns: 1fr 1fr;
    gap: 0.75rem;
    margin-top: 0.75rem;
  }

  @media (max-width: 768px) {
    .aegis-grid {
      grid-template-columns: 1fr;
    }
  }

  .aegis-card {
    background: rgba(0, 0, 0, 0.22);
    border: 1px solid var(--border-color);
    border-radius: 6px;
    padding: 0.75rem;
    display: flex;
    flex-direction: column;
    gap: 0.45rem;
  }

  .aegis-card.synthetic-detected {
    border-color: rgba(239, 68, 68, 0.45);
    background: rgba(239, 68, 68, 0.05);
  }

  .aegis-card-top {
    display: flex;
    justify-content: space-between;
    align-items: flex-start;
  }

  .aegis-card-title {
    display: flex;
    align-items: flex-start;
    gap: 0.5rem;
  }

  .aegis-icon {
    font-size: 1.3rem;
    line-height: 1;
  }

  .aegis-card-title h4 {
    margin: 0;
    font-size: 0.85rem;
    font-weight: 700;
    color: var(--text-heading);
  }

  .aegis-class-label {
    display: block;
    font-size: 0.72rem;
    color: var(--text-muted);
    margin-top: 0.15rem;
  }

  .aegis-score-box {
    text-align: right;
  }

  .aegis-score-num {
    display: block;
    font-size: 1.25rem;
    font-weight: 800;
    line-height: 1.1;
    color: var(--text-heading);
  }

  .score-high .aegis-score-num {
    color: #ef4444;
  }

  .score-med .aegis-score-num {
    color: #f59e0b;
  }

  .aegis-score-label {
    font-size: 0.62rem;
    text-transform: uppercase;
    color: var(--text-muted);
  }

  .aegis-meter-track {
    width: 100%;
    height: 6px;
    border-radius: 3px;
    background: rgba(255, 255, 255, 0.08);
    overflow: hidden;
  }

  .aegis-meter-fill {
    height: 100%;
    background: #3b82f6;
    border-radius: 3px;
    transition: width 0.3s ease;
  }

  .aegis-meter-fill.fill-alert {
    background: linear-gradient(90deg, #f59e0b, #ef4444);
  }

  .aegis-attribution-box {
    margin-top: 0.2rem;
  }

  .aegis-target-alert {
    display: flex;
    align-items: center;
    gap: 0.5rem;
    background: rgba(239, 68, 68, 0.16);
    border: 1px solid rgba(239, 68, 68, 0.45);
    padding: 0.45rem 0.6rem;
    border-radius: 5px;
    color: #fca5a5;
  }

  .target-badge-icon {
    font-size: 1.1rem;
  }

  .target-badge-info {
    font-size: 0.75rem;
    display: flex;
    flex-direction: column;
  }

  .target-name {
    font-size: 0.85rem;
    font-weight: 800;
    color: #fee2e2;
  }

  .target-conf {
    font-size: 0.68rem;
    color: #fca5a5;
  }

  .aegis-target-unattributed {
    font-size: 0.72rem;
    color: #d1d5db;
    background: rgba(255, 255, 255, 0.04);
    padding: 0.35rem 0.5rem;
    border-radius: 4px;
  }

  .aegis-target-clean {
    font-size: 0.72rem;
    color: #10b981;
    padding: 0.25rem 0.4rem;
  }

  .aegis-anomalies-list {
    display: flex;
    align-items: center;
    flex-wrap: wrap;
    gap: 0.3rem;
    margin-top: 0.3rem;
  }

  .anomalies-head {
    font-size: 0.68rem;
    font-weight: 700;
    color: var(--text-muted);
  }

  .aegis-anomaly-chip {
    font-size: 0.65rem;
    font-weight: 600;
    padding: 2px 6px;
    border-radius: 4px;
    background: rgba(239, 68, 68, 0.15);
    color: #fca5a5;
    border: 1px solid rgba(239, 68, 68, 0.3);
  }

  .aegis-metrics-grid {
    display: flex;
    flex-wrap: wrap;
    gap: 0.65rem;
    margin-top: 0.35rem;
    padding-top: 0.35rem;
    border-top: 1px dashed rgba(255, 255, 255, 0.08);
  }

  .metric-item {
    font-size: 0.68rem;
    display: flex;
    gap: 0.25rem;
  }

  .m-label {
    color: var(--text-muted);
  }

  .m-val {
    font-weight: 700;
    color: var(--text-body);
  }

  .aegis-multimodal-bar {
    display: flex;
    justify-content: space-between;
    align-items: center;
    padding: 0.6rem 0.75rem;
    margin-top: 0.75rem;
    background: rgba(0, 0, 0, 0.22);
    border: 1px solid var(--border-color);
    border-radius: 6px;
    gap: 0.75rem;
  }

  .aegis-multimodal-bar.av-mismatch {
    border-color: rgba(239, 68, 68, 0.4);
    background: rgba(239, 68, 68, 0.08);
  }

  .av-sync-info {
    display: flex;
    gap: 0.6rem;
    align-items: center;
    font-size: 0.78rem;
  }

  .av-icon {
    font-size: 1.2rem;
  }

  .av-desc {
    display: block;
    font-size: 0.7rem;
    color: var(--text-muted);
    margin-top: 0.1rem;
  }

  .av-score-pill {
    font-size: 0.72rem;
    font-weight: 700;
    padding: 2px 8px;
    border-radius: 999px;
    background: rgba(255, 255, 255, 0.08);
    color: var(--text-body);
    white-space: nowrap;
  }

  .aegis-summary-box {
    margin-top: 0.75rem;
    padding: 0.65rem 0.85rem;
    background: rgba(0, 0, 0, 0.25);
    border-left: 3px solid #6366f1;
    border-radius: 4px;
  }

  .summary-title {
    font-size: 0.75rem;
    font-weight: 700;
    color: #a5b4fc;
    display: block;
    margin-bottom: 0.25rem;
  }

  .aegis-summary-box p {
    margin: 0;
    font-size: 0.78rem;
    line-height: 1.45;
    color: var(--text-body);
  }

  .aegis-empty-state {
    padding: 1.2rem 0.5rem;
    text-align: center;
    color: var(--text-muted);
    font-size: 0.82rem;
  }

  .aegis-error-banner {
    margin: 0.5rem 0;
    padding: 0.4rem 0.6rem;
    background: rgba(239, 68, 68, 0.15);
    border: 1px solid #ef4444;
    color: #fca5a5;
    font-size: 0.75rem;
    border-radius: 4px;
  }

  .transcript-btn-group {
    display: flex;
    align-items: center;
    flex-wrap: wrap;
    gap: 0.35rem;
  }

  .copy-toast {
    font-size: 0.72rem;
    font-weight: 700;
    color: var(--verdict-supported);
    background: rgba(16, 185, 129, 0.15);
    border: 1px solid rgba(16, 185, 129, 0.3);
    padding: 2px 6px;
    border-radius: 4px;
    white-space: nowrap;
  }

  .transcript-editor {
    width: 100%;
    padding: 0.5rem;
    font-size: 0.85rem;
    background: var(--input-bg);
    border: 1px solid var(--border-color);
    border-radius: 6px;
    color: var(--text-body);
    box-sizing: border-box;
    font-family: inherit;
  }

  .transcript-segments {
    font-size: 0.88rem;
    line-height: 1.6;
    background: var(--surface-bg);
    padding: 0.75rem;
    border-radius: 6px;
    border: 1px solid var(--border-color);
  }

  .transcript-segment {
    cursor: pointer;
    padding: 2px 4px;
    border-radius: 4px;
    transition: background 0.15s ease, border-color 0.15s ease;
    display: inline;
  }

  .transcript-segment:hover {
    background: var(--hover-bg);
  }

  .transcript-segment.active {
    background: var(--primary-accent-alpha);
    color: var(--primary-accent);
    font-weight: 600;
  }

  .transcript-segment.claim-mark-contradicted {
    background: rgba(225, 29, 72, 0.15);
    border-bottom: 2px solid var(--verdict-contradicted);
    color: var(--text-heading);
  }

  .transcript-segment.claim-mark-supported {
    background: rgba(16, 185, 129, 0.15);
    border-bottom: 2px solid var(--verdict-supported);
    color: var(--text-heading);
  }

  .transcript-segment.claim-mark-unverifiable {
    background: rgba(217, 119, 6, 0.15);
    border-bottom: 2px solid var(--verdict-unverifiable);
    color: var(--text-heading);
  }

  .ocr-overview {
    margin-bottom: 0.6rem;
    padding: 0.55rem 0.75rem;
    background: var(--surface-bg);
    border: 1px solid var(--border-color);
    border-radius: 6px;
  }

  .ocr-overview-head {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 0.75rem;
    flex-wrap: wrap;
  }

  .ocr-toggle {
    font-size: 0.78rem;
    font-weight: 600;
    padding: 3px 10px;
    border-radius: 999px;
    border: 1px solid var(--border-color);
    background: var(--pill-bg);
    color: var(--text-muted);
    cursor: pointer;
    transition: background 0.15s ease, color 0.15s ease, border-color 0.15s ease;
  }

  .ocr-toggle:hover {
    background: var(--hover-bg);
  }

  .ocr-toggle.on {
    background: var(--primary-accent-alpha);
    color: var(--primary-accent);
    border-color: var(--primary-accent);
  }

  .ocr-legend {
    display: inline-flex;
    align-items: center;
    gap: 0.4rem;
    font-size: 0.72rem;
    color: var(--text-muted);
  }

  .ocr-legend i {
    display: inline-block;
    border-radius: 3px;
  }

  .ocr-legend .lg-speech {
    width: 16px;
    height: 6px;
    background: var(--text-muted);
    opacity: 0.4;
  }

  .ocr-legend .lg-ocr {
    width: 4px;
    height: 12px;
    margin-left: 0.5rem;
    background: var(--primary-accent);
  }

  .ocr-track {
    position: relative;
    height: 22px;
    margin-top: 0.55rem;
    background: var(--input-bg);
    border-radius: 4px;
  }

  .ocr-track-speech {
    position: absolute;
    top: 8px;
    height: 6px;
    border-radius: 3px;
    background: var(--text-muted);
    opacity: 0.4;
  }

  .ocr-track-mark {
    position: absolute;
    top: 2px;
    width: 4px;
    height: 18px;
    padding: 0;
    border: 0;
    border-radius: 2px;
    transform: translateX(-50%);
    background: var(--primary-accent);
    cursor: pointer;
    transition: transform 0.12s ease;
  }

  .ocr-track-mark:hover {
    transform: translateX(-50%) scaleX(1.8);
  }

  .ocr-track-playhead {
    position: absolute;
    top: 0;
    bottom: 0;
    width: 2px;
    background: var(--text-heading);
    opacity: 0.55;
    pointer-events: none;
  }

  .ocr-chip {
    display: flex;
    align-items: baseline;
    gap: 0.5rem;
    margin: 0.35rem 0;
    padding: 0.3rem 0.6rem;
    border-left: 3px solid var(--primary-accent);
    border-radius: 4px;
    background: var(--primary-accent-alpha);
    color: var(--text-body);
    font-size: 0.82rem;
    line-height: 1.45;
    cursor: pointer;
    transition: background 0.15s ease, box-shadow 0.15s ease;
  }

  .ocr-chip:hover {
    background: var(--hover-bg);
  }

  .ocr-chip.active {
    box-shadow: 0 0 0 1px var(--primary-accent);
    color: var(--text-heading);
  }

  .ocr-chip-tag {
    flex-shrink: 0;
    font-size: 0.7rem;
    font-weight: 700;
    letter-spacing: 0.02em;
    color: var(--primary-accent);
    white-space: nowrap;
  }

  .ocr-chip-text {
    font-style: italic;
    word-break: break-word;
  }

  .ocr-tail {
    margin-top: -1px;
    padding: 0.4rem 0.75rem;
    background: var(--surface-bg);
    border: 1px solid var(--border-color);
    border-top: 1px dashed var(--border-color);
    border-radius: 0 0 6px 6px;
  }

  .claim-badge-inline {
    font-size: 0.68rem;
    font-weight: 700;
    padding: 1px 4px;
    border-radius: 3px;
    margin-right: 2px;
    cursor: pointer;
    background: var(--primary-accent-alpha);
    color: var(--primary-accent);
  }

  .claim-badge-inline.contradicted {
    background: rgba(225, 29, 72, 0.25);
    color: var(--verdict-contradicted);
  }

  .claim-badge-inline.supported {
    background: rgba(16, 185, 129, 0.25);
    color: var(--verdict-supported);
  }

  .claim-badge-inline.unverifiable {
    background: rgba(217, 119, 6, 0.25);
    color: var(--verdict-unverifiable);
  }

  .highlighted-pulse {
    animation: segPulse 1.8s ease-in-out;
  }

  @keyframes segPulse {
    0% {
      box-shadow: 0 0 0 0 rgba(37, 99, 235, 0.8);
      background: rgba(37, 99, 235, 0.35);
    }
    50% {
      box-shadow: 0 0 0 6px rgba(37, 99, 235, 0);
      background: rgba(37, 99, 235, 0.45);
    }
    100% {
      box-shadow: 0 0 0 0 rgba(37, 99, 235, 0);
    }
  }

  .claim-block {
    background: var(--surface-bg);
    border: 1px solid var(--border-color);
    border-left: 4px solid var(--border-color);
    border-radius: 6px;
    padding: 0.85rem;
    margin-bottom: 0.85rem;
    transition: box-shadow 0.2s ease, border-color 0.2s ease;
  }

  .claim-block.claim-verdict-supported {
    border-left-color: #10b981;
    background: linear-gradient(90deg, rgba(16, 185, 129, 0.045) 0%, var(--surface-bg) 32%);
  }

  .claim-block.claim-verdict-contradicted {
    border-left-color: #ef4444;
    background: linear-gradient(90deg, rgba(239, 68, 68, 0.05) 0%, var(--surface-bg) 32%);
  }

  .claim-block.claim-verdict-unverifiable {
    border-left-color: #eab308;
    background: linear-gradient(90deg, rgba(234, 179, 8, 0.045) 0%, var(--surface-bg) 32%);
  }

  .claim-pulse {
    border-color: var(--primary-accent) !important;
    box-shadow: 0 0 14px rgba(37, 99, 235, 0.3) !important;
  }

  .claim-header {
    display: flex;
    align-items: center;
    flex-wrap: wrap;
    gap: 0.4rem;
    margin-bottom: 0.5rem;
  }

  .claim-index-pill {
    font-size: 0.7rem;
    font-weight: 800;
    background: var(--pill-bg);
    color: var(--text-heading);
    padding: 2px 6px;
    border-radius: 4px;
    border: 1px solid var(--border-color);
  }

  .claim-source-tag {
    font-size: 0.68rem;
    font-weight: 700;
    background: var(--pill-bg);
    color: var(--text-muted);
    padding: 1px 6px;
    border-radius: 4px;
    white-space: nowrap;
  }

  .claim-source-tag.screen,
  .claim-source-tag.both {
    background: var(--primary-accent-alpha);
    color: var(--primary-accent);
  }

  .claim-type-tag {
    font-size: 0.68rem;
    font-weight: 700;
    text-transform: uppercase;
    background: var(--pill-bg);
    color: var(--text-muted);
    padding: 1px 5px;
    border-radius: 4px;
  }

  .verdict-tag {
    font-size: 0.72rem;
    font-weight: 700;
    padding: 2px 7px;
    border-radius: 999px;
  }

  .verdict-tag.contradicted {
    background: rgba(225, 29, 72, 0.15);
    color: var(--verdict-contradicted);
  }

  .verdict-tag.supported {
    background: rgba(16, 185, 129, 0.15);
    color: var(--verdict-supported);
  }

  .verdict-tag.unverifiable,
  .verdict-tag.already_checked {
    background: rgba(217, 119, 6, 0.15);
    color: var(--verdict-unverifiable);
  }

  .btn-shortcut {
    font-size: 0.72rem;
    font-weight: 700;
    padding: 2px 7px;
    border-radius: 4px;
    background: var(--primary-accent-alpha);
    color: var(--primary-accent);
    border: 1px solid rgba(37, 99, 235, 0.25);
    cursor: pointer;
  }

  .btn-shortcut:hover {
    background: var(--primary-accent);
    color: #fff;
  }

  .claim-text {
    font-size: 0.9rem;
    font-weight: 600;
    color: var(--text-heading);
    margin-bottom: 0.55rem;
    line-height: 1.45;
  }

  .attribution-box {
    background: var(--card-bg);
    border-left: 3px solid var(--primary-accent);
    padding: 0.45rem 0.65rem;
    border-radius: 0 4px 4px 0;
    margin-bottom: 0.55rem;
    font-size: 0.8rem;
  }

  .attr-row {
    display: flex;
    gap: 0.35rem;
    margin-bottom: 0.2rem;
  }

  .attr-label {
    font-weight: 700;
    color: var(--text-muted);
  }

  .attr-val {
    color: var(--text-body);
  }

  .claim-reasoning {
    font-size: 0.81rem;
    color: var(--text-body);
    margin-bottom: 0.7rem;
    line-height: 1.45;
    background: var(--card-bg);
    padding: 0.6rem 0.75rem;
    border-radius: 6px;
    border: 1px solid var(--border-color);
    border-left: 3px solid var(--border-color);
    display: flex;
    flex-direction: column;
    gap: 0.5rem;
  }

  .claim-reasoning.reasoning-verdict-supported {
    border-left-color: #10b981;
    border-color: rgba(16, 185, 129, 0.28);
    background: rgba(16, 185, 129, 0.04);
  }

  .claim-reasoning.reasoning-verdict-contradicted {
    border-left-color: #ef4444;
    border-color: rgba(239, 68, 68, 0.3);
    background: rgba(239, 68, 68, 0.045);
  }

  .claim-reasoning.reasoning-verdict-unverifiable {
    border-left-color: #eab308;
    border-color: rgba(234, 179, 8, 0.3);
    background: rgba(234, 179, 8, 0.04);
  }

  .reasoning-head {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 0.5rem;
    padding-bottom: 0.3rem;
    border-bottom: 1px solid rgba(148, 163, 184, 0.16);
  }

  .reasoning-title {
    font-size: 0.78rem;
    font-weight: 800;
    text-transform: uppercase;
    letter-spacing: 0.02em;
    color: var(--text-heading);
  }

  .reasoning-count-pill {
    font-size: 0.68rem;
    font-weight: 700;
    padding: 1px 7px;
    border-radius: 999px;
    background: var(--pill-bg);
    color: var(--text-muted);
    border: 1px solid var(--border-color);
  }

  .reasoning-statements-list {
    display: flex;
    flex-direction: column;
    gap: 0.42rem;
  }

  .detail-drawer.statement-grid-mode .reasoning-statements-list,
  .detail-drawer.ultrawide-mode .reasoning-statements-list {
    display: grid;
    grid-template-columns: repeat(auto-fit, minmax(var(--statement-card-min, 250px), 1fr));
    gap: 0.48rem;
  }

  .reasoning-stmt-card {
    padding: 0.45rem 0.6rem;
    border-radius: 5px;
    border: 1px solid var(--border-color);
    border-left: 3px solid var(--border-color);
    background: var(--surface-bg);
    display: flex;
    flex-direction: column;
    gap: 0.25rem;
  }

  .reasoning-stmt-card.stmt-supported {
    border-left-color: #10b981;
    border-color: rgba(16, 185, 129, 0.32);
    background: rgba(16, 185, 129, 0.075);
  }

  .reasoning-stmt-card.stmt-contradicted {
    border-left-color: #ef4444;
    border-color: rgba(239, 68, 68, 0.35);
    background: rgba(239, 68, 68, 0.08);
  }

  .reasoning-stmt-card.stmt-unverifiable {
    border-left-color: #eab308;
    border-color: rgba(234, 179, 8, 0.35);
    background: rgba(234, 179, 8, 0.075);
  }

  .stmt-top-row {
    display: flex;
    align-items: center;
    flex-wrap: wrap;
    gap: 0.35rem;
  }

  .stmt-index {
    font-size: 0.68rem;
    font-weight: 800;
    color: var(--text-muted);
  }

  .stmt-status-pill {
    font-size: 0.68rem;
    font-weight: 800;
    padding: 1px 6px;
    border-radius: 4px;
    text-transform: uppercase;
    letter-spacing: 0.01em;
  }

  .stmt-status-pill.supported {
    background: rgba(16, 185, 129, 0.2);
    color: #10b981;
  }

  .stmt-status-pill.contradicted {
    background: rgba(239, 68, 68, 0.2);
    color: #ef4444;
  }

  .stmt-status-pill.unverifiable {
    background: rgba(234, 179, 8, 0.22);
    color: #d97706;
  }

  :global([data-theme='dark']) .stmt-status-pill.unverifiable {
    color: #fbbf24;
  }

  .stmt-subject {
    font-size: 0.8rem;
    font-weight: 700;
    color: var(--text-heading);
  }

  .stmt-refs {
    display: inline-flex;
    align-items: center;
    gap: 0.25rem;
    margin-left: auto;
  }

  .stmt-ref-btn {
    font-size: 0.68rem;
    font-weight: 700;
    padding: 0 5px;
    border-radius: 3px;
    border: 1px solid rgba(59, 130, 246, 0.35);
    background: rgba(59, 130, 246, 0.12);
    color: var(--primary-accent);
    cursor: pointer;
  }

  .stmt-ref-btn:hover {
    background: var(--primary-accent);
    color: #fff;
  }

  .stmt-ref-tag {
    font-size: 0.68rem;
    font-weight: 700;
    color: var(--text-muted);
  }

  .stmt-detail {
    font-size: 0.79rem;
    color: var(--text-body);
    line-height: 1.42;
  }

  .reasoning-summary-footer {
    padding: 0.5rem 0.65rem;
    border-radius: 5px;
    background: var(--surface-bg);
    border: 1px solid var(--border-color);
    border-top: 2px solid var(--border-color);
    font-size: 0.81rem;
    line-height: 1.45;
  }

  .reasoning-summary-footer.summary-supported {
    border-top-color: #10b981;
    background: rgba(16, 185, 129, 0.09);
  }

  .reasoning-summary-footer.summary-contradicted {
    border-top-color: #ef4444;
    background: rgba(239, 68, 68, 0.095);
  }

  .reasoning-summary-footer.summary-unverifiable {
    border-top-color: #eab308;
    background: rgba(234, 179, 8, 0.09);
  }

  .summary-badge {
    font-weight: 800;
    color: var(--text-heading);
    margin-right: 0.3rem;
  }

  .summary-text {
    color: var(--text-body);
  }

  .reasoning-extra-notes {
    display: flex;
    flex-direction: column;
    gap: 0.25rem;
    font-size: 0.76rem;
    color: var(--text-muted);
  }

  .reasoning-extra-note {
    padding: 0.3rem 0.5rem;
    border-radius: 4px;
    background: rgba(148, 163, 184, 0.08);
    border: 1px dashed var(--border-color);
  }

  .factcheck-action-bar {
    background: var(--card-bg);
    border: 1px solid var(--border-color);
    border-radius: 6px;
    padding: 0.55rem 0.65rem;
    margin-bottom: 0.65rem;
  }

  .factcheck-bar-title {
    display: block;
    font-size: 0.74rem;
    font-weight: 700;
    color: var(--text-muted);
    text-transform: uppercase;
    margin-bottom: 0.4rem;
  }

  .factcheck-links-grid {
    display: flex;
    flex-wrap: wrap;
    gap: 0.35rem;
  }

  .factcheck-link-btn {
    font-size: 0.74rem;
    font-weight: 600;
    padding: 3px 8px;
    border-radius: 4px;
    text-decoration: none;
    display: inline-flex;
    align-items: center;
    border: 1px solid var(--border-color);
    background: var(--surface-bg);
    color: var(--text-body);
    transition: all 0.12s ease;
  }

  .factcheck-link-btn:hover {
    background: var(--hover-bg);
    color: var(--text-heading);
    border-color: var(--primary-accent);
  }

  .factcheck-bg {
    color: #0284c7;
    border-color: rgba(2, 132, 199, 0.3);
  }

  .factcheck-google {
    color: #4f46e5;
    border-color: rgba(79, 70, 229, 0.3);
  }

  .factcheck-bta {
    color: #059669;
    border-color: rgba(5, 150, 105, 0.3);
  }

  .factcheck-verify-btn {
    font-size: 0.74rem;
    font-weight: 700;
  }

  .evidence-list h4 {
    margin: 0.5rem 0 0.35rem 0;
    font-size: 0.8rem;
    font-weight: 700;
    color: var(--text-muted);
    text-transform: uppercase;
  }

  .evidence-empty-box {
    display: flex;
    align-items: center;
    justify-content: space-between;
    background: var(--card-bg);
    border: 1px dashed var(--border-color);
    padding: 0.5rem 0.65rem;
    border-radius: 4px;
    gap: 0.5rem;
  }

  .evidence-items-grid {
    display: flex;
    flex-direction: column;
    gap: 0.4rem;
  }

  .detail-drawer.evidence-grid-mode .evidence-items-grid {
    display: grid;
    grid-template-columns: repeat(auto-fit, minmax(var(--evidence-card-min, 260px), 1fr));
    gap: 0.5rem;
  }

  .evidence-item {
    background: var(--card-bg);
    border: 1px solid var(--border-color);
    border-radius: 6px;
    padding: 0.45rem 0.65rem;
    margin-bottom: 0;
    font-size: 0.78rem;
    transition: all 0.25s ease-in-out;
  }

  .evidence-item.evidence-pulse {
    border-color: #3b82f6 !important;
    box-shadow: 0 0 16px rgba(59, 130, 246, 0.45) !important;
    transform: scale(1.01);
  }

  :global(mark.matched-passage) {
    background-color: rgba(234, 179, 8, 0.28);
    color: inherit;
    font-weight: 600;
    padding: 0 2px;
    border-radius: 2px;
  }

  .claim-feedback-bar {
    display: flex;
    align-items: center;
    gap: 0.4rem;
    margin: 0.5rem 0;
    padding: 0.35rem 0.6rem;
    background: rgba(255, 255, 255, 0.04);
    border: 1px solid var(--border-color, rgba(255, 255, 255, 0.1));
    border-radius: 6px;
    font-size: 0.8rem;
    flex-wrap: wrap;
  }

  .feedback-hint {
    color: var(--text-secondary, #94a3b8);
    font-weight: 500;
  }

  .btn-feedback {
    background: transparent;
    border: 1px solid rgba(255, 255, 255, 0.15);
    border-radius: 4px;
    padding: 2px 7px;
    font-size: 0.76rem;
    cursor: pointer;
    color: inherit;
    transition: all 0.15s ease;
  }

  .btn-feedback:hover {
    background: rgba(255, 255, 255, 0.08);
  }

  .btn-feedback.selected.correct {
    background: rgba(16, 185, 129, 0.2);
    border-color: #10b981;
    color: #34d399;
  }

  .btn-feedback.selected.wrong {
    background: rgba(239, 68, 68, 0.2);
    border-color: #ef4444;
    color: #f87171;
  }

  .btn-feedback.selected.unrelated {
    background: rgba(245, 158, 11, 0.2);
    border-color: #f59e0b;
    color: #fbbf24;
  }

  .feedback-saved-badge {
    color: #34d399;
    font-size: 0.76rem;
    font-weight: 600;
    margin-left: 0.2rem;
  }

  .feedback-verdict-picker {
    display: flex;
    align-items: center;
    gap: 0.35rem;
    margin: 0.3rem 0 0.5rem 0.5rem;
    font-size: 0.78rem;
  }

  .picker-label {
    color: var(--text-secondary, #94a3b8);
  }

  .evidence-connection-bar {
    display: flex;
    align-items: center;
    flex-wrap: wrap;
    gap: 0.35rem;
    margin-bottom: 0.35rem;
    padding-bottom: 0.3rem;
    border-bottom: 1px solid rgba(255, 255, 255, 0.05);
  }

  .source-category-pill {
    font-size: 0.68rem;
    font-weight: 700;
    padding: 1px 6px;
    border-radius: 4px;
    display: inline-flex;
    align-items: center;
    gap: 0.2rem;
  }

  .cat-primary {
    background: rgba(59, 130, 246, 0.15);
    color: #60a5fa;
    border: 1px solid rgba(59, 130, 246, 0.3);
  }

  .cat-factcheck {
    background: rgba(168, 85, 247, 0.15);
    color: #c084fc;
    border: 1px solid rgba(168, 85, 247, 0.3);
  }

  .cat-news {
    background: rgba(245, 158, 11, 0.15);
    color: #fbbf24;
    border: 1px solid rgba(245, 158, 11, 0.3);
  }

  .cat-other {
    background: rgba(148, 163, 184, 0.12);
    color: #94a3b8;
    border: 1px solid rgba(148, 163, 184, 0.25);
  }

  .connection-strength-pill {
    font-size: 0.68rem;
    font-weight: 600;
    color: var(--text-muted);
    background: rgba(255, 255, 255, 0.04);
    padding: 1px 6px;
    border-radius: 4px;
    margin-left: auto;
  }

  .evidence-match-keywords {
    display: flex;
    align-items: center;
    flex-wrap: wrap;
    gap: 0.25rem;
    margin: 0.25rem 0 0.4rem 0;
    padding: 0.25rem 0.4rem;
    background: rgba(255, 255, 255, 0.02);
    border-radius: 4px;
  }

  .keywords-label {
    font-size: 0.68rem;
    color: var(--text-muted);
    font-weight: 600;
  }

  .keyword-pill {
    font-size: 0.65rem;
    font-weight: 600;
    background: rgba(99, 102, 241, 0.18);
    color: #a5b4fc;
    padding: 1px 5px;
    border-radius: 3px;
  }

  .delete-evidence-btn {
    font-size: 0.68rem;
    padding: 0.15rem 0.45rem;
    color: var(--text-muted);
    border-color: rgba(255, 255, 255, 0.1);
  }

  .delete-evidence-btn:hover {
    color: #ef4444;
    border-color: rgba(239, 68, 68, 0.4);
    background: rgba(239, 68, 68, 0.1);
  }

  .evidence-meta {
    display: flex;
    align-items: center;
    gap: 0.35rem;
    margin-bottom: 0.25rem;
  }

  .stance-badge {
    font-weight: 700;
    padding: 1px 5px;
    border-radius: 3px;
    font-size: 0.68rem;
  }

  .stance-contradicts {
    background: rgba(225, 29, 72, 0.15);
    color: var(--verdict-contradicted);
  }

  .stance-supports {
    background: rgba(16, 185, 129, 0.15);
    color: var(--verdict-supported);
  }

  .stance-neutral {
    background: rgba(100, 116, 139, 0.15);
    color: #64748b;
  }

  .evidence-source {
    font-weight: 600;
    color: var(--text-body);
  }

  .evidence-date {
    color: var(--text-muted);
  }

  .evidence-snippet {
    color: var(--text-body);
    line-height: 1.35;
    margin-bottom: 0.25rem;
  }

  .evidence-link {
    color: var(--primary-accent);
    text-decoration: none;
    font-weight: 600;
  }

  .evidence-link:hover {
    text-decoration: underline;
  }

  .people-grid {
    display: flex;
    flex-direction: column;
    gap: 0.65rem;
  }

  .detail-drawer.evidence-grid-mode .people-grid {
    display: grid;
    grid-template-columns: repeat(auto-fill, minmax(var(--evidence-card-min, 255px), 1fr));
    gap: 0.6rem;
  }

  .person-card {
    background: var(--surface-bg);
    border: 1px solid var(--border-color);
    border-radius: 6px;
    padding: 0.65rem 0.8rem;
    display: flex;
    flex-direction: column;
    gap: 0.45rem;
  }

  .person-card-header {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 0.5rem;
  }

  .person-name-group {
    display: flex;
    flex-direction: column;
    gap: 0.2rem;
  }

  .person-title-row {
    display: flex;
    align-items: center;
    gap: 0.45rem;
    flex-wrap: wrap;
  }

  .person-name {
    font-weight: 700;
    color: var(--text-heading);
    font-size: 0.88rem;
  }

  .person-entity-badge {
    font-size: 0.65rem;
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
    background: rgba(14, 165, 233, 0.12);
    color: #0284c7;
    border: 1px solid rgba(14, 165, 233, 0.3);
  }

  .person-entity-badge.type-building {
    background: rgba(13, 148, 136, 0.12);
    color: #0d9488;
    border: 1px solid rgba(13, 148, 136, 0.3);
  }

  .person-role {
    font-size: 0.74rem;
    color: var(--text-muted);
  }

  .person-cred-badge {
    font-size: 0.7rem;
    font-weight: 700;
    padding: 2px 7px;
    border-radius: 999px;
  }

  .cred-low {
    background: rgba(225, 29, 72, 0.15);
    color: var(--verdict-contradicted);
  }

  .cred-high {
    background: rgba(16, 185, 129, 0.15);
    color: var(--verdict-supported);
  }

  .cred-moderate {
    background: rgba(217, 119, 6, 0.15);
    color: var(--verdict-unverifiable);
  }

  .cred-neutral {
    background: rgba(100, 116, 139, 0.15);
    color: #64748b;
  }

  .credibility-meter {
    background: var(--card-bg);
    padding: 0.4rem 0.55rem;
    border-radius: 5px;
    border: 1px solid var(--border-color);
  }

  .meter-bar-track {
    width: 100%;
    height: 6px;
    background: var(--border-color);
    border-radius: 999px;
    overflow: hidden;
    margin-bottom: 0.25rem;
  }

  .meter-bar-fill {
    height: 100%;
    border-radius: 999px;
    transition: width 0.3s ease;
  }

  .meter-bar-fill.cred-low {
    background: var(--verdict-contradicted);
  }

  .meter-bar-fill.cred-high {
    background: var(--verdict-supported);
  }

  .meter-bar-fill.cred-moderate {
    background: var(--verdict-unverifiable);
  }

  .meter-bar-fill.cred-neutral {
    background: #64748b;
  }

  .meter-labels {
    display: flex;
    justify-content: space-between;
    font-size: 0.72rem;
    color: var(--text-muted);
  }

  .person-actions-row {
    display: flex;
    flex-wrap: wrap;
    gap: 0.35rem;
    align-items: center;
  }

  .person-link-btn {
    font-size: 0.72rem;
    font-weight: 600;
    padding: 2px 7px;
    border-radius: 4px;
    text-decoration: none;
    border: 1px solid var(--border-color);
    background: var(--card-bg);
    color: var(--text-body);
    transition: all 0.12s ease;
  }

  .person-link-btn:hover {
    border-color: var(--primary-accent);
    color: var(--text-heading);
  }

  .wikidata-link {
    color: var(--primary-accent);
  }

  .person-verify-btn {
    font-size: 0.72rem;
    font-weight: 700;
  }

  .creator-box {
    display: flex;
    flex-direction: column;
    gap: 0.45rem;
    font-size: 0.85rem;
    background: var(--surface-bg);
    padding: 0.75rem;
    border-radius: 6px;
    border: 1px solid var(--border-color);
    margin-bottom: 0.75rem;
  }

  .creator-main-row {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 0.5rem;
  }

  .creator-name-link {
    font-weight: 800;
    font-size: 0.95rem;
    color: var(--primary-accent);
    text-decoration: none;
  }

  .creator-name-link:hover {
    text-decoration: underline;
  }

  .creator-sub {
    color: var(--text-muted);
    font-size: 0.8rem;
    margin-left: 0.3rem;
  }

  .creator-stat-row {
    font-size: 0.78rem;
    color: var(--text-muted);
    display: flex;
    gap: 0.5rem;
    flex-wrap: wrap;
  }

  .author-risk-header-badges {
    display: flex;
    align-items: center;
    gap: 0.5rem;
    flex-wrap: wrap;
  }

  .author-risk-badge {
    font-size: 0.72rem;
    font-weight: 800;
    padding: 2px 8px;
    border-radius: 999px;
  }

  .author-confidence-pill {
    font-size: 0.72rem;
    font-weight: 700;
    padding: 2px 8px;
    border-radius: 999px;
    cursor: help;
  }

  .conf-high {
    background: rgba(16, 185, 129, 0.15);
    color: #10b981;
    border: 1px solid rgba(16, 185, 129, 0.3);
  }

  .conf-moderate {
    background: rgba(217, 119, 6, 0.15);
    color: #d97706;
    border: 1px solid rgba(217, 119, 6, 0.3);
  }

  .conf-low {
    background: rgba(225, 29, 72, 0.15);
    color: #e11d48;
    border: 1px solid rgba(225, 29, 72, 0.3);
  }

  .risk-high {
    background: rgba(225, 29, 72, 0.18);
    color: var(--verdict-contradicted);
    border: 1px solid rgba(225, 29, 72, 0.35);
  }

  .risk-moderate {
    background: rgba(217, 119, 6, 0.18);
    color: var(--verdict-unverifiable);
    border: 1px solid rgba(217, 119, 6, 0.35);
  }

  .risk-low {
    background: rgba(16, 185, 129, 0.18);
    color: var(--verdict-supported);
    border: 1px solid rgba(16, 185, 129, 0.35);
  }

  .risk-unverified {
    background: rgba(148, 163, 184, 0.15);
    color: #94a3b8;
    border: 1px solid rgba(148, 163, 184, 0.3);
  }

  .btn-web-risk {
    border-color: rgba(99, 102, 241, 0.4);
    color: #818cf8;
  }

  .btn-web-risk:hover:not(:disabled) {
    background: rgba(99, 102, 241, 0.15);
    border-color: #818cf8;
  }

  .author-risk-assessment-card {
    margin-top: 0.75rem;
    padding: 0.75rem;
    background: rgba(15, 23, 42, 0.55);
    border: 1px solid rgba(255, 255, 255, 0.08);
    border-radius: 8px;
    display: flex;
    flex-direction: column;
    gap: 0.55rem;
  }

  .risk-assessment-top {
    display: flex;
    align-items: center;
    justify-content: space-between;
  }

  .risk-assessment-title {
    font-size: 0.82rem;
    font-weight: 700;
    color: var(--text-heading);
  }

  .risk-calc-time {
    font-size: 0.7rem;
    color: var(--text-muted);
  }

  .risk-rationale-text {
    margin: 0;
    font-size: 0.78rem;
    line-height: 1.35;
    color: var(--text-body);
  }

  .confidence-bar-container {
    display: flex;
    flex-direction: column;
    gap: 0.25rem;
    background: rgba(0, 0, 0, 0.2);
    padding: 0.45rem 0.6rem;
    border-radius: 6px;
  }

  .confidence-bar-label {
    display: flex;
    justify-content: space-between;
    font-size: 0.72rem;
    color: var(--text-muted);
  }

  .confidence-bar-track {
    height: 6px;
    background: rgba(255, 255, 255, 0.08);
    border-radius: 999px;
    overflow: hidden;
  }

  .confidence-bar-fill {
    height: 100%;
    border-radius: 999px;
    transition: width 0.3s ease;
  }

  .confidence-bar-fill.conf-high {
    background: #10b981;
  }

  .confidence-bar-fill.conf-moderate {
    background: #d97706;
  }

  .confidence-bar-fill.conf-low {
    background: #e11d48;
  }

  .confidence-reason-hint {
    font-size: 0.7rem;
    color: var(--text-muted);
  }

  .author-web-sources-section {
    display: flex;
    flex-direction: column;
    gap: 0.35rem;
  }

  .web-sources-heading {
    font-size: 0.74rem;
    font-weight: 700;
    color: var(--text-heading);
  }

  .web-sources-chips {
    display: flex;
    flex-wrap: wrap;
    gap: 0.4rem;
  }

  .web-source-chip {
    display: inline-flex;
    align-items: center;
    gap: 0.35rem;
    padding: 3px 8px;
    background: rgba(30, 41, 59, 0.7);
    border: 1px solid rgba(255, 255, 255, 0.1);
    border-radius: 6px;
    font-size: 0.72rem;
    color: var(--text-body);
    text-decoration: none;
    transition: all 0.2s ease;
    max-width: 100%;
  }

  .web-source-chip:hover {
    border-color: #818cf8;
    background: rgba(99, 102, 241, 0.15);
  }

  .source-domain-tag {
    font-weight: 700;
    color: #818cf8;
  }

  .source-title-text {
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    max-width: 260px;
  }

  .author-other-header {
    display: flex;
    align-items: center;
    justify-content: space-between;
    margin-bottom: 0.45rem;
  }

  .author-other-header h4 {
    margin: 0;
    font-size: 0.82rem;
    font-weight: 700;
    color: var(--text-heading);
  }

  .other-videos-grid {
    display: flex;
    flex-direction: column;
    gap: 0.4rem;
  }

  .other-video-card {
    background: var(--surface-bg);
    border: 1px solid var(--border-color);
    border-radius: 5px;
    padding: 0.45rem 0.65rem;
    cursor: pointer;
    transition: background 0.12s ease, border-color 0.12s ease;
  }

  .other-video-card:hover {
    background: var(--hover-bg);
    border-color: var(--primary-accent);
  }

  .other-vid-top {
    display: flex;
    align-items: center;
    justify-content: space-between;
    margin-bottom: 0.25rem;
  }

  .other-vid-id {
    font-size: 0.7rem;
    color: var(--text-muted);
  }

  .other-vid-claim {
    font-size: 0.8rem;
    font-weight: 600;
    color: var(--text-heading);
    margin-bottom: 0.3rem;
    display: -webkit-box;
    -webkit-line-clamp: 2;
    line-clamp: 2;
    -webkit-box-orient: vertical;
    overflow: hidden;
    line-height: 1.35;
  }

  .other-vid-stats {
    display: flex;
    align-items: center;
    gap: 0.5rem;
    font-size: 0.72rem;
    color: var(--text-muted);
  }

  .inspect-btn {
    margin-left: auto;
    color: var(--primary-accent);
    font-weight: 700;
  }

  .collapsible-header {
    width: 100%;
    display: flex;
    align-items: center;
    justify-content: space-between;
    background: none;
    border: none;
    font-size: 0.85rem;
    font-weight: 700;
    color: var(--text-heading);
    cursor: pointer;
    padding: 0;
  }

  .metadata-dump {
    margin-top: 0.5rem;
    padding: 0.5rem;
    background: var(--input-bg);
    border: 1px solid var(--border-color);
    border-radius: 4px;
    font-size: 0.72rem;
    overflow-x: auto;
  }

  .empty-hint {
    color: var(--text-muted);
    font-size: 0.8rem;
    margin: 0;
  }

  .drawer-header-actions {
    display: flex;
    align-items: center;
    gap: 0.5rem;
  }

  .copy-all-btn {
    font-size: 0.76rem;
    font-weight: 600;
    white-space: nowrap;
    padding: 0.25rem 0.65rem;
    display: inline-flex;
    align-items: center;
    gap: 0.35rem;
    background: rgba(59, 130, 246, 0.15);
    color: #60a5fa;
    border: 1px solid rgba(59, 130, 246, 0.35);
    border-radius: 4px;
    cursor: pointer;
    transition: all 0.15s ease;
  }

  .copy-all-btn:hover {
    background: rgba(59, 130, 246, 0.25);
    border-color: rgba(59, 130, 246, 0.55);
    color: #93c5fd;
  }

  .drawer-remove-btn {
    font-size: 0.76rem;
    padding: 0.25rem 0.6rem;
    background: rgba(225, 29, 72, 0.2);
    color: var(--verdict-contradicted);
    border: 1px solid rgba(225, 29, 72, 0.4);
    border-radius: 4px;
    cursor: pointer;
  }

  .drawer-remove-btn:hover {
    background: rgba(225, 29, 72, 0.35);
  }

  .google-sge-btn {
    text-decoration: none;
    color: #60a5fa !important;
    border-color: rgba(96, 165, 250, 0.4);
  }

  .google-sge-btn:hover {
    background: rgba(96, 165, 250, 0.15);
  }

  .preview-act-btn {
    font-size: 0.68rem;
    padding: 0.15rem 0.45rem;
    margin-left: auto;
  }

  .evidence-actions-row {
    display: flex;
    align-items: center;
    justify-content: space-between;
    margin-top: 0.4rem;
  }

  .toggle-expand-btn {
    font-size: 0.7rem;
    color: var(--primary-accent);
    cursor: pointer;
  }

  .person-bio-box {
    margin: 0.4rem 0;
    padding: 0.45rem 0.65rem;
    background: rgba(255, 255, 255, 0.03);
    border-left: 2px solid var(--primary-accent);
    border-radius: 0 4px 4px 0;
  }

  .person-bio-text {
    margin: 0;
    font-size: 0.78rem;
    line-height: 1.4;
    color: var(--text-body);
  }

  .person-stats-mini {
    display: flex;
    gap: 0.5rem;
    font-size: 0.7rem;
    color: var(--text-muted);
    margin-top: 0.25rem;
  }

  .stat-warn {
    color: var(--verdict-contradicted);
    font-weight: 600;
  }

  .stat-ok {
    color: var(--verdict-supported);
    font-weight: 600;
  }

  .creator-empty-box {
    display: flex;
    flex-direction: column;
    gap: 0.5rem;
    margin-top: 0.25rem;
  }

  .creator-empty-actions {
    display: flex;
    gap: 0.5rem;
    flex-wrap: wrap;
  }

  .modal-backdrop {
    position: fixed;
    top: 0;
    left: 0;
    width: 100vw;
    height: 100vh;
    background: rgba(0, 0, 0, 0.65);
    backdrop-filter: blur(4px);
    z-index: 1000;
    display: flex;
    align-items: center;
    justify-content: center;
    padding: 1.5rem;
  }

  .preview-modal {
    background: var(--surface-bg, #1a1a1a);
    border: 1px solid var(--border-color, #333);
    border-radius: 8px;
    width: 100%;
    max-width: 720px;
    max-height: 85vh;
    display: flex;
    flex-direction: column;
    overflow: hidden;
    box-shadow: 0 20px 40px rgba(0, 0, 0, 0.5);
  }

  :global([data-widescreen='true']) .preview-modal {
    max-width: min(90vw, 1320px);
  }

  .preview-modal-header {
    display: flex;
    align-items: flex-start;
    justify-content: space-between;
    padding: 1rem 1.25rem;
    border-bottom: 1px solid var(--border-color, #333);
  }

  .preview-domain-pill {
    font-size: 0.7rem;
    background: rgba(59, 130, 246, 0.2);
    color: #60a5fa;
    padding: 0.15rem 0.45rem;
    border-radius: 4px;
    font-weight: 600;
  }

  .preview-modal-title {
    margin: 0.35rem 0 0 0;
    font-size: 1.05rem;
    font-weight: 700;
    color: var(--text-heading);
    line-height: 1.35;
  }

  .preview-modal-body {
    padding: 1.25rem;
    overflow-y: auto;
    flex: 1;
    font-size: 0.88rem;
    line-height: 1.6;
    color: var(--text-body);
  }

  .preview-paragraph {
    margin: 0 0 0.85rem 0;
  }

  .preview-modal-footer {
    display: flex;
    align-items: center;
    justify-content: space-between;
    padding: 0.85rem 1.25rem;
    border-top: 1px solid var(--border-color, #333);
    background: rgba(0, 0, 0, 0.15);
  }

  .google-ai-citations-box {
    margin-top: 1.25rem;
    padding-top: 1rem;
    border-top: 1px solid var(--border-color, #333);
  }

  .citations-title {
    margin: 0 0 0.5rem 0;
    font-size: 0.82rem;
    font-weight: 700;
    color: var(--text-heading);
  }

  .citations-list {
    margin: 0;
    padding-left: 1.2rem;
    font-size: 0.8rem;
    display: flex;
    flex-direction: column;
    gap: 0.35rem;
  }

  .citation-link {
    color: var(--primary-accent, #3b82f6);
    text-decoration: none;
    word-break: break-all;
  }

  .citation-link:hover {
    text-decoration: underline;
  }
  .claim-redact-box { display: flex; flex-direction: column; gap: 6px; margin: 8px 0; }
  .claim-redact-box textarea { width: 100%; padding: 6px; border-radius: 6px; background: var(--bg-elevated, #1e1e1e); color: inherit; border: 1px solid #444; }
  .redact-actions { display: flex; gap: 6px; }
</style>
