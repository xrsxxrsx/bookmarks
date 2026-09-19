import { useCallback, useRef, useState } from 'react';

import { highlight, languageBadge } from '../format';
import type { Row, ViewOptions } from '../library';

interface Props {
  rows: Row[];
  selectedId: number | null;
  onSelect: (id: number) => void;
  options: ViewOptions;
  busy: boolean;
  /** Filters by a tag clicked in a row. */
  onTagClick: (tag: string) => void;
}

/** Column keys, in display order. Used for both the header and the width overrides. */
const COLUMNS = [
  { key: 'status', label: '', cls: 'col-status' },
  { key: 'title', label: '标题', cls: 'col-title' },
  { key: 'author', label: '作者', cls: 'col-author' },
  { key: 'lang', label: '语言', cls: 'col-lang' },
  { key: 'words', label: '字数', cls: 'col-words' },
  { key: 'date', label: '发布', cls: 'col-date' },
  { key: 'tags', label: '我的标签', cls: 'col-tags' },
] as const;

type ColumnKey = (typeof COLUMNS)[number]['key'];

/**
 * Explicit column widths, set by dragging a header edge.
 *
 * Only columns the user actually dragged appear here; the rest stay `auto` so the browser
 * keeps distributing space sensibly at other window sizes. A fixed width for every column
 * from the start would freeze the layout at whatever the first render happened to compute.
 *
 * Widths are stored in `rem` so they scale with the interface font size, and are kept in
 * memory rather than persisted: they are a per-session adjustment, and a stale saved width
 * is more annoying than not remembering it.
 */
function useColumnWidths() {
  const [widths, setWidths] = useState<Partial<Record<ColumnKey, number>>>({});
  const dragging = useRef<{ key: ColumnKey; startX: number; startRem: number } | null>(null);

  const start = useCallback(
    (key: ColumnKey, event: React.PointerEvent<HTMLSpanElement>) => {
      const header = event.currentTarget.parentElement;
      if (header === null) return;
      const rootPx = Number.parseFloat(
        getComputedStyle(document.documentElement).fontSize || '14',
      );
      const currentPx = header.getBoundingClientRect().width;
      dragging.current = { key, startX: event.clientX, startRem: currentPx / rootPx };
      event.currentTarget.setPointerCapture(event.pointerId);
      event.preventDefault();
      event.stopPropagation();
    },
    [],
  );

  const move = useCallback((event: React.PointerEvent<HTMLSpanElement>) => {
    const drag = dragging.current;
    if (drag === null) return;
    const rootPx = Number.parseFloat(getComputedStyle(document.documentElement).fontSize || '14');
    const deltaRem = (event.clientX - drag.startX) / rootPx;
    // Floors keep a column from being dragged away entirely.
    const minimums: Record<ColumnKey, number> = {
      status: 1.6,
      title: 8,
      author: 5,
      lang: 2.5,
      words: 4,
      date: 4.5,
      tags: 6,
    };
    const next = Math.max(minimums[drag.key], drag.startRem + deltaRem);
    setWidths((prev) => ({ ...prev, [drag.key]: next }));
  }, []);

  const end = useCallback((event: React.PointerEvent<HTMLSpanElement>) => {
    if (dragging.current === null) return;
    dragging.current = null;
    if (event.currentTarget.hasPointerCapture(event.pointerId)) {
      event.currentTarget.releasePointerCapture(event.pointerId);
    }
  }, []);

  const reset = useCallback(() => setWidths({}), []);

  return { widths, start, move, end, reset, hasOverrides: Object.keys(widths).length > 0 };
}

export function WorkTable({ rows, selectedId, onSelect, options, busy, onTagClick }: Props) {
  const { widths, start, move, end, reset, hasOverrides } = useColumnWidths();

  if (rows.length === 0) {
    return (
      <div className="table-wrap">
        <div className="empty">
          <p>没有符合条件的作品。</p>
          <p className="muted">点右上角「导入文件」或「导入文件夹」把已下载的 AO3 文件放进来。</p>
        </div>
      </div>
    );
  }

  const required = options.requiredTags.map((t) => t.toLowerCase());

  return (
    <div className="table-wrap">
      {hasOverrides && (
        <button type="button" className="ghost small reset-columns" onClick={reset}>
          重置列宽
        </button>
      )}
      <table className={busy ? 'busy' : ''}>
        <colgroup>
          {COLUMNS.map((column) => {
            const rem = widths[column.key];
            return <col key={column.key} style={rem === undefined ? undefined : { width: `${rem}rem` }} />;
          })}
        </colgroup>
        <thead>
          <tr>
            {COLUMNS.map((column) => (
              <th key={column.key} className={column.cls}>
                <span className="th-label">{column.label}</span>
                <span
                  className="col-resize"
                  title="拖动调整列宽"
                  onPointerDown={(e) => start(column.key, e)}
                  onPointerMove={move}
                  onPointerUp={end}
                  onPointerCancel={end}
                  onClick={(e) => e.stopPropagation()}
                />
              </th>
            ))}
          </tr>
        </thead>
        <tbody>
          {rows.map((row) => (
            <WorkRow
              key={row.work.id}
              row={row}
              selected={row.work.id === selectedId}
              query={options.search}
              requiredTags={required}
              onSelect={onSelect}
              onTagClick={onTagClick}
            />
          ))}
        </tbody>
      </table>
    </div>
  );
}

interface RowProps {
  row: Row;
  selected: boolean;
  query: string;
  requiredTags: string[];
  onSelect: (id: number) => void;
  onTagClick: (tag: string) => void;
}

function WorkRow({ row, selected, query, requiredTags, onSelect, onTagClick }: RowProps) {
  const { work, depth, groupSize } = row;

  const classes = ['work-row'];
  if (selected) classes.push('selected');
  if (depth > 0) classes.push('nested');
  if (work.status === 'record_only') classes.push('record-only');

  const missingFiles = work.files.some((f) => !f.exists);

  return (
    <tr
      className={classes.join(' ')}
      onClick={() => onSelect(work.id)}
      // Keyboard-reachable, since the row is the main way into a work.
      tabIndex={0}
      onKeyDown={(e) => {
        if (e.key === 'Enter' || e.key === ' ') {
          e.preventDefault();
          onSelect(work.id);
        }
      }}
    >
      <td className="col-status" title={work.status === 'has_file' ? '有文件' : '仅记录（还没有下载文件）'}>
        <span className={work.status === 'has_file' ? 'status-dot on' : 'status-dot off'} />
      </td>
      <td className="col-title">
        <div className="title-cell">
          {/*
            Two reserved slots rather than one shared box: the tree marker and the
            translation badge each get their own fixed space, present or not. Sharing one
            fixed box failed in practice — a flex item will not shrink below its content by
            default, so the badge widened the box on exactly the rows that had one, and
            those were the rows whose titles shifted.
          */}
          <span className="tree-slot">
            {depth > 0 ? (
              <span className="branch" aria-label="关联作品">
                └
              </span>
            ) : (
              groupSize > 1 && (
                <span className="branch-root" title={`与 ${groupSize - 1} 篇关联作品分为一组`}>
                  ▾
                </span>
              )
            )}
          </span>
          <span className="badge-slot">
            {work.translation_role === 'translation' && (
              <span className="chip translation" title="这是一篇译文">
                译
              </span>
            )}
          </span>

          <span className="title-text">
            <Highlighted text={work.title} query={query} />
          </span>

          <span className="title-flags">
            {missingFiles && (
              <span className="chip warn" title="有文件在磁盘上找不到了">
                文件缺失
              </span>
            )}
            {work.files.length > 1 && (
              <span className="chip subtle" title="这篇文有多个格式">
                {work.files.length} 格式
              </span>
            )}
          </span>
        </div>
      </td>
      <td className="col-author">
        <Highlighted text={work.author ?? '—'} query={query} />
      </td>
      <td className="col-lang" title={work.language_label ?? ''}>
        <span className="lang-badge">{languageBadge(work.language)}</span>
      </td>
      <td className="col-words num" title={work.word_count_source ?? ''}>
        {work.word_count_display}
      </td>
      <td className="col-date num" title={dateTitle(work.published_prec, work.date_is_approx)}>
        {work.published_display ?? '—'}
      </td>
      <td className="col-tags">
        {work.tags.length === 0 ? (
          <span className="muted">—</span>
        ) : (
          work.tags.map((tag) => {
            const active = requiredTags.includes(tag.toLowerCase());
            return (
              <button
                key={tag}
                type="button"
                className={active ? 'chip tag clickable active' : 'chip tag clickable'}
                title={active ? `取消筛选「${tag}」` : `只看带「${tag}」的作品`}
                onClick={(e) => {
                  // Filtering must not also select the row, which would swap the detail panel.
                  e.stopPropagation();
                  onTagClick(tag);
                }}
              >
                <Highlighted text={tag} query={query} />
              </button>
            );
          })
        )}
      </td>
    </tr>
  );
}

/** Explains how much of the date is actually known. */
function dateTitle(precision: string | null, approx: boolean): string {
  const parts: string[] = [];
  if (precision === 'year') parts.push('只知道年份');
  else if (precision === 'month') parts.push('只知道年月');
  else if (precision === 'day') parts.push('完整日期');
  else parts.push('没有日期');
  if (approx) parts.push('数值为估算');
  return parts.join('，');
}

function Highlighted({ text, query }: { text: string; query: string }) {
  const parts = highlight(text, query);
  if (parts.length === 1 && !parts[0]?.hit) return <>{text}</>;
  return (
    <>
      {parts.map((part, i) =>
        part.hit ? <mark key={i}>{part.text}</mark> : <span key={i}>{part.text}</span>,
      )}
    </>
  );
}
