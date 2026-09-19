import { useEffect, useMemo, useState } from 'react';

import * as api from '../api';
import { formatBytes, formatLabel, languageBadge } from '../format';
import type { Library, Work, WorkUpdate } from '../types';

interface Props {
  work: Work | null;
  library: Library;
  busy: boolean;
  onSave: (update: WorkUpdate) => Promise<void>;
  /** Replaces the work's series membership, creating series as needed. */
  onSaveSeries: (
    workId: number,
    series: { name: string; position: number | null }[],
  ) => Promise<void>;
  onCopyAo3Tags: (workId: number) => Promise<void>;
  /** Deletes the work, its files and its directory. Irreversible from here. */
  onDelete: (workId: number) => Promise<void>;
  onSelectWork: (id: number) => void;
}

export function WorkDetail(props: Props) {
  const { work } = props;
  if (work === null) {
    return (
      <aside className="detail empty-detail">
        <p className="muted">从左边选一篇作品查看和编辑详情。</p>
      </aside>
    );
  }
  // Remount on selection change so the draft state resets to the newly selected work
  // instead of carrying edits across.
  return <WorkForm key={work.id} {...props} work={work} />;
}

/** The editable state of a work, as strings, so the form never holds parsed values. */
interface Draft {
  title: string;
  author: string;
  summary: string;
  comment: string;
  tags: string[];
  series: { name: string; position: string }[];
  /** ISO 639-1 code. Editable because a bare TXT or a DOCX carries no language metadata. */
  language: string;
  /** Free text as well as a number: the count is often pasted with thousands separators. */
  wordCount: string;
  published: string;
  publishedApprox: boolean;
  completed: string;
  completedApprox: boolean;
  status: string;
}

/**
 * The date shown in the edit box.
 *
 * The stored value is a full ISO date, but showing `2019-01-01` for a work whose year is
 * only known would assert a day that is not real. Only the known components are shown, so
 * re-saving without touching the field is a no-op rather than a silent precision upgrade.
 */
function draftDate(iso: string | null, precision: string | null): string {
  if (iso === null) return '';
  if (precision === 'year') return iso.slice(0, 4);
  if (precision === 'month') return iso.slice(0, 7);
  return iso.slice(0, 10);
}

function toDraft(work: Work): Draft {
  return {
    title: work.title,
    author: work.author ?? '',
    summary: work.summary ?? '',
    comment: work.my_comment ?? '',
    tags: work.tags,
    series: work.series.map((s) => ({
      name: s.name,
      position: s.position === null ? '' : String(s.position),
    })),
    language: work.language ?? '',
    wordCount: work.word_count > 0 ? String(work.word_count) : '',
    published: draftDate(work.published_at, work.published_prec),
    publishedApprox: work.date_is_approx,
    completed: draftDate(work.completed_at, work.completed_prec),
    completedApprox: work.completed_is_approx,
    status: work.status,
  };
}

function sameDraft(a: Draft, b: Draft): boolean {
  return (
    a.title === b.title &&
    a.author === b.author &&
    a.summary === b.summary &&
    a.comment === b.comment &&
    a.published === b.published &&
    a.publishedApprox === b.publishedApprox &&
    a.completed === b.completed &&
    a.completedApprox === b.completedApprox &&
    a.status === b.status &&
    a.language === b.language &&
    a.wordCount === b.wordCount &&
    a.tags.join('\u0000') === b.tags.join('\u0000') &&
    a.series.map((s) => `${s.name}\u0001${s.position}`).join('\u0000') ===
      b.series.map((s) => `${s.name}\u0001${s.position}`).join('\u0000')
  );
}

function WorkForm({
  work,
  library,
  busy,
  onSave,
  onSaveSeries,
  onCopyAo3Tags,
  onDelete,
  onSelectWork,
}: Props & { work: Work }) {
  const original = useMemo(() => toDraft(work), [work]);
  const [draft, setDraft] = useState<Draft>(original);
  const [tagDraft, setTagDraft] = useState('');
  const [seriesDraft, setSeriesDraft] = useState('');
  const [dateError, setDateError] = useState<string | null>(null);
  // Deletion is irreversible from inside the application, so it takes a second, explicit click
  // rather than a browser confirm dialog — the dialog can be dismissed by reflex, this cannot.
  const [confirmingDelete, setConfirmingDelete] = useState(false);

  // A saved work comes back as a new object; resync so the dirty check clears.
  useEffect(() => {
    setDraft(original);
    setDateError(null);
  }, [original]);

  const dirty = !sameDraft(draft, original);

  function patch(changes: Partial<Draft>): void {
    setDraft((prev) => ({ ...prev, ...changes }));
  }

  function addTag(): void {
    const value = tagDraft.trim();
    if (value === '') return;
    if (!draft.tags.some((t) => t.toLowerCase() === value.toLowerCase())) {
      patch({ tags: [...draft.tags, value].sort((a, b) => a.localeCompare(b)) });
    }
    setTagDraft('');
  }

  /** Adds a series membership. A name already present is ignored rather than duplicated. */
  function addSeries(): void {
    const value = seriesDraft.trim();
    if (value === '') return;
    if (!draft.series.some((s) => s.name.toLowerCase() === value.toLowerCase())) {
      patch({ series: [...draft.series, { name: value, position: '' }] });
    }
    setSeriesDraft('');
  }

  async function save(): Promise<void> {
    setDateError(null);
    try {
      /*
       * Only the fields that actually changed are sent, which gives dates three distinct
       * meanings rather than two:
       *   omitted  -> leave whatever is stored alone
       *   an object -> set this value (an empty input clears it)
       * A form that always sent both dates could not express "leave this one alone", so an
       * unrelated edit would have silently wiped a date the user never touched.
       */
      const update: WorkUpdate = {
        work_id: work.id,
        title: draft.title,
        author: draft.author,
        summary: draft.summary,
        my_comment: draft.comment,
        tags: draft.tags,
        status: draft.status,
      };
      if (draft.published !== original.published || draft.publishedApprox !== original.publishedApprox) {
        update.published = { input: draft.published, approx: draft.publishedApprox };
      }
      if (draft.completed !== original.completed || draft.completedApprox !== original.completedApprox) {
        update.completed = { input: draft.completed, approx: draft.completedApprox };
      }
      /*
       * Language and word count are only sent when touched, for the same reason as the dates: an
       * untouched empty field must mean "no change", not "clear it". Both are stored with their
       * provenance, so a typed count is shown as a fact rather than an estimate.
       */
      if (draft.language !== original.language) {
        const code = draft.language.trim().toLowerCase();
        update.language = code;
        update.language_label = LANGUAGE_LABELS[code] ?? code;
      }
      if (draft.wordCount !== original.wordCount) {
        // Tolerate a pasted count with separators.
        const digits = draft.wordCount.replace(/[^\d]/g, '');
        update.word_count = digits === '' ? 0 : Number.parseInt(digits, 10);
      }

      await onSave(update);

      // Series membership is a separate call: it creates series rows, so it is not part of the
      // work's own columns.
      const seriesChanged =
        draft.series.map((s) => `${s.name}\u0001${s.position}`).join('\u0000') !==
        original.series.map((s) => `${s.name}\u0001${s.position}`).join('\u0000');
      if (seriesChanged) {
        const payload = draft.series
          .filter((s) => s.name.trim() !== '')
          .map((s) => {
            const parsed = Number.parseInt(s.position, 10);
            return {
              name: s.name.trim(),
              position: Number.isFinite(parsed) ? parsed : null,
            };
          });
        await onSaveSeries(work.id, payload);
      }
    } catch (e: unknown) {
      const message = e instanceof Error ? e.message : String(e);
      if (message.includes('日期')) setDateError(message);
      throw e;
    }
  }

  const related = work.related.map((r) => ({
    ...r,
    partner: library.works.find((w) => w.id === r.work_id) ?? null,
  }));

  return (
    <aside className="detail">
      {/*
        One editable block rather than a heading plus a separate form: the values shown at
        the top *are* the fields. A separate read-only header meant every fact appeared
        twice and only one copy could be edited.
      */}
      <div className="detail-form">
        <label className="field title-field">
          <span>标题</span>
          <input
            className="title-input"
            value={draft.title}
            onChange={(e) => patch({ title: e.target.value })}
          />
        </label>

        <div className="field-grid">
          {/*
            The author spans the full width. A pseud like
            "AnnaBabalaCarlton (seeseaEve)" does not fit in a third of the panel, and
            squeezing it into one column wrapped it onto three lines.
          */}
          <label className="field span-all">
            <span>作者</span>
            <input
              value={draft.author}
              placeholder="（未知）"
              onChange={(e) => patch({ author: e.target.value })}
            />
          </label>

          {/*
            Status, language and word count share one row, and all three are deliberately short:
            a dot, an ISO code, a number. Anything longer wraps and breaks the row's alignment —
            AO3's Chinese label `中文-普通话 國語` is long enough to do exactly that, so it is kept as
            the tooltip rather than as visible text. The full label is still stored and still
            reachable on hover.
          */}
          <label className="field readonly-field" title="有文件 / 仅记录">
            <span>状态</span>
            <select value={draft.status} onChange={(e) => patch({ status: e.target.value })}>
              <option value="has_file">有文件</option>
              <option value="record_only">仅记录</option>
            </select>
          </label>

          <label className="field readonly-field" title={work.language_label ?? '语言未知'}>
            <span>语言</span>
            <input
              className="short-input"
              value={draft.language}
              placeholder="zh / en"
              list="known-languages"
              maxLength={8}
              onChange={(e) => patch({ language: e.target.value })}
            />
          </label>

          <label className="field readonly-field" title={wordCountTitle(work.word_count_source)}>
            <span>字数</span>
            <input
              className="short-input"
              value={draft.wordCount}
              placeholder="留空＝未知"
              inputMode="numeric"
              onChange={(e) => patch({ wordCount: e.target.value })}
            />
          </label>

          <datalist id="known-languages">
            {KNOWN_LANGUAGES.map((code) => (
              <option key={code} value={code} />
            ))}
          </datalist>

          {/*
            The "约" checkbox sits beside the label rather than beside the input. Next to the
            input it competed for the same narrow row and wrapped onto its own line at this
            panel width.
          */}
          <label className="field">
            <span className="field-label-row">
              <span>发布时间</span>
              <span className="approx-toggle" title="这个日期是估的，不是来源明确给出的">
                <input
                  type="checkbox"
                  checked={draft.publishedApprox}
                  onChange={(e) => patch({ publishedApprox: e.target.checked })}
                />
                约
              </span>
            </span>
            <input
              value={draft.published}
              placeholder="2019 / 2019-06-09"
              onChange={(e) => patch({ published: e.target.value })}
            />
          </label>

          {/*
            A completion date is optional: many works are unfinished, abandoned, or only
            have an approximate date. An empty box means exactly that, and the placeholder
            says so rather than implying the field is required.
          */}
          <label className="field">
            <span className="field-label-row">
              <span>完结时间</span>
              <span className="approx-toggle" title="这个日期是估的">
                <input
                  type="checkbox"
                  checked={draft.completedApprox}
                  onChange={(e) => patch({ completedApprox: e.target.checked })}
                />
                约
              </span>
            </span>
            <input
              value={draft.completed}
              placeholder="留空＝未完结或未知"
              onChange={(e) => patch({ completed: e.target.value })}
            />
          </label>
        </div>

        {dateError !== null && <div className="inline-error">{dateError}</div>}

        <div className="field">
          <span>
            我的标签
            {draft.tags.length > 0 && <span className="muted">（点击表格里的标签可直接筛选）</span>}
          </span>
          <div className="tag-editor">
            {draft.tags.map((tag) => (
              <span key={tag} className="chip tag removable">
                {tag}
                <button
                  type="button"
                  className="chip-remove"
                  title="移除"
                  onClick={() => patch({ tags: draft.tags.filter((t) => t !== tag) })}
                >
                  ×
                </button>
              </span>
            ))}
            <input
              className="tag-input"
              value={tagDraft}
              placeholder="输入 cp 名后回车"
              onChange={(e) => setTagDraft(e.target.value)}
              onKeyDown={(e) => {
                if (e.key === 'Enter') {
                  e.preventDefault();
                  addTag();
                }
              }}
              onBlur={addTag}
            />
          </div>
        </div>

        {/*
          Series membership. Kept to a name plus an optional position because that is all the
          relation actually needs: a work belongs to a series, and the position says where it
          falls in the reading order. The input suggests names already in use, so typing an
          existing one joins it rather than creating a near-duplicate.
        */}
        <div className="field">
          <span>
            系列
            {draft.series.length > 0 && (
              <span className="muted">（数字是系列内的顺序，可留空）</span>
            )}
          </span>
          <div className="series-editor">
            {draft.series.map((entry, index) => (
              <div className="series-row" key={`${entry.name}-${index}`}>
                <span className="chip series">{entry.name}</span>
                <input
                  className="series-position"
                  value={entry.position}
                  placeholder="序"
                  inputMode="numeric"
                  title="系列内的顺序"
                  onChange={(e) =>
                    patch({
                      series: draft.series.map((s, i) =>
                        i === index ? { ...s, position: e.target.value } : s,
                      ),
                    })
                  }
                />
                <span className="muted">
                  共 {library.works.filter((w) => w.series.some((s) => s.name === entry.name)).length} 篇
                </span>
                <button
                  type="button"
                  className="chip-remove"
                  title="移出这个系列"
                  onClick={() =>
                    patch({ series: draft.series.filter((_, i) => i !== index) })
                  }
                >
                  ×
                </button>
              </div>
            ))}
            <input
              className="tag-input"
              list="known-series"
              value={seriesDraft}
              placeholder="输入系列名后回车；同名会并入同一个系列"
              onChange={(e) => setSeriesDraft(e.target.value)}
              onKeyDown={(e) => {
                if (e.key === 'Enter') {
                  e.preventDefault();
                  addSeries();
                }
              }}
              onBlur={addSeries}
            />
            <datalist id="known-series">
              {library.all_series.map((name) => (
                <option key={name} value={name} />
              ))}
            </datalist>
          </div>
        </div>

        <label className="field">
          <span>Summary</span>
          <textarea rows={5} value={draft.summary} onChange={(e) => patch({ summary: e.target.value })} />
        </label>
        <label className="field">
          <span>我的评论</span>
          <textarea
            rows={4}
            value={draft.comment}
            placeholder="记点什么，比如为什么收藏、看到哪了"
            onChange={(e) => patch({ comment: e.target.value })}
          />
        </label>

        <div className="detail-actions">
          <button type="button" onClick={() => void save()} disabled={!dirty || busy}>
            保存
          </button>
          {dirty && (
            <>
              <button type="button" className="ghost" disabled={busy} onClick={() => setDraft(original)}>
                放弃修改
              </button>
              <span className="muted">有未保存的修改</span>
            </>
          )}
        </div>
      </div>

      {work.files.length > 0 && (
        <section className="detail-section">
          <h3>文件</h3>
          <ul className="file-list">
            {work.files.map((file) => (
              <li key={file.id} className={file.exists ? '' : 'missing'}>
                <span className="file-format">{formatLabel(file)}</span>
                <span className="file-name" title={file.original_name}>
                  {file.original_name}
                </span>
                <span className="muted">{formatBytes(file.size_bytes)}</span>
                {file.exists ? (
                  <>
                    <button type="button" className="small" onClick={() => void api.openFile(file.id)}>
                      打开
                    </button>
                    <button
                      type="button"
                      className="ghost small"
                      onClick={() => void api.revealFile(file.id)}
                    >
                      定位
                    </button>
                  </>
                ) : (
                  <span className="chip warn">文件缺失</span>
                )}
              </li>
            ))}
          </ul>
        </section>
      )}

      {related.length > 0 && (
        <section className="detail-section">
          <h3>关联作品</h3>
          <ul className="related-list">
            {related.map((rel) => (
              <li key={rel.work_id}>
                <span className="chip translation">
                  {rel.kind === 'translation' ? '译本' : rel.kind}
                </span>
                <button
                  type="button"
                  className="link-button"
                  onClick={() => onSelectWork(rel.work_id)}
                >
                  {rel.title}
                </button>
                {rel.partner !== null && (
                  <span className="lang-badge">{languageBadge(rel.partner.language)}</span>
                )}
                {rel.author !== null && <span className="muted">by {rel.author}</span>}
                {rel.origin === 'manual' && <span className="chip subtle">手动关联</span>}
              </li>
            ))}
          </ul>
        </section>
      )}

      {work.ao3_tags.length > 0 && (
        <section className="detail-section">
          <h3>
            AO3 官方标签 <span className="muted">({work.ao3_tags.length})</span>
            <button
              type="button"
              className="ghost small"
              disabled={busy}
              title="把这些标签复制到「我的标签」，已存在的不会重复"
              onClick={() => void onCopyAo3Tags(work.id)}
            >
              加入我的标签
            </button>
          </h3>
          <div className="tag-cloud">
            {work.ao3_tags.map((tag) => (
              <span key={tag} className="chip ao3">
                {tag}
              </span>
            ))}
          </div>
        </section>
      )}

      {(work.source_url !== null || work.external_id !== null) && (
        <section className="detail-section">
          <h3>来源</h3>
          {work.external_id !== null && (
            <a
              href={`https://archiveofourown.org/works/${work.external_id}`}
              target="_blank"
              rel="noreferrer"
              className="link"
            >
              AO3 /{work.external_id}
            </a>
          )}
        </section>
      )}

      <section className="detail-section danger-zone">
        {!confirmingDelete ? (
          <button
            type="button"
            className="ghost small"
            disabled={busy}
            title="删除这篇作品"
            onClick={() => setConfirmingDelete(true)}
          >
            删除这篇作品…
          </button>
        ) : (
          <div className="delete-confirm">
            <p>
              将删除这条记录
              {work.files.length > 0 && (
                <>
                  及其 <b>{work.files.length}</b> 个文件
                </>
              )}
              ，并从磁盘上移除
              {work.files.length > 0 ? '它们' : '它的目录'}。
              <br />
              <span className="muted">
                书库目录之外没有副本，删除后无法从这里恢复。
              </span>
            </p>
            <div className="delete-actions">
              <button
                type="button"
                className="danger"
                disabled={busy}
                onClick={() => void onDelete(work.id)}
              >
                确认删除
              </button>
              <button
                type="button"
                className="ghost"
                disabled={busy}
                onClick={() => setConfirmingDelete(false)}
              >
                取消
              </button>
            </div>
          </div>
        )}
      </section>
    </aside>
  );
}

function wordCountTitle(source: string | null): string {
  if (source === 'ao3') return 'AO3 给出的官方字数，精确';
  if (source === 'manual') return '你手动填写的字数';
  if (source === 'estimated') return '按文本估算的字数，与 AO3 官方值可能有偏差';
  return '字数未知，可以自己填';
}

/**
 * The languages worth offering as autocomplete.
 *
 * Only codes, because that is what is stored and what the list shows; the display label is derived
 * from this table. Anything not here is still accepted and stored as typed, so a rare language is
 * never blocked — the list is a convenience, not a constraint.
 */
const LANGUAGE_LABELS: Record<string, string> = {
  zh: '中文-普通话 國語',
  en: 'English',
  ja: '日本語',
  ko: '한국어',
  fr: 'Français',
  de: 'Deutsch',
  es: 'Español',
  ru: 'Русский',
  pt: 'Português',
  it: 'Italiano',
};

const KNOWN_LANGUAGES = Object.keys(LANGUAGE_LABELS);
