import { useCallback, useEffect, useMemo, useRef, useState } from 'react';

import { languageFacets, type GroupMode, type SortField, type ViewOptions } from '../library';
import type { Work } from '../types';

interface Props {
  options: ViewOptions;
  onChange: (next: ViewOptions) => void;
  /** The user's tags with how many works carry each, derived from the works. */
  tags: { name: string; count: number }[];
  works: Work[];
  disabled: boolean;
}

const SORT_LABELS: Record<SortField, string> = {
  published: '发布时间',
  words: '字数',
  added: '加入顺序',
  title: '标题',
};

export function FilterBar({
  options,
  onChange,
  tags,
  works,
  disabled,
}: Props) {
  const [searchDraft, setSearchDraft] = useState(options.search);

  // Debounce so typing does not re-filter on every keystroke. The whole library is in
  // memory, but re-rendering a few hundred rows per keypress is still visible.
  useEffect(() => {
    const timer = setTimeout(() => {
      if (searchDraft !== options.search) {
        onChange({ ...options, search: searchDraft });
      }
    }, 120);
    return () => clearTimeout(timer);
  }, [searchDraft, options, onChange]);

  const languages = useMemo(() => languageFacets(works), [works]);

  function toggleTag(tag: string): void {
    const has = options.requiredTags.some((t) => t.toLowerCase() === tag.toLowerCase());
    onChange({
      ...options,
      requiredTags: has
        ? options.requiredTags.filter((t) => t.toLowerCase() !== tag.toLowerCase())
        : [...options.requiredTags, tag],
    });
  }

  function toggleLanguage(code: string): void {
    const has = options.languages.includes(code);
    onChange({
      ...options,
      languages: has ? options.languages.filter((l) => l !== code) : [...options.languages, code],
    });
  }

  const filtersActive =
    options.search !== '' ||
    options.requiredTags.length > 0 ||
    options.languages.length > 0 ||
    !options.includeRecordOnly;

  return (
    <div className="filterbar">
      <input
        type="search"
        className="search"
        placeholder="搜索标题 / 作者 / 标签 / 评论 / 简介"
        value={searchDraft}
        disabled={disabled}
        onChange={(e) => setSearchDraft(e.target.value)}
        aria-label="搜索"
      />

      <Dropdown label="标签" count={options.requiredTags.length}>
        {(close) => (
          <div className="dropdown-list">
            {tags.length === 0 && (
              <div className="muted pad">还没有标签。在右侧详情里给作品加标签后会出现在这里。</div>
            )}
            {tags.map((tag) => (
              <label key={tag.name} className="dropdown-item">
                <input
                  type="checkbox"
                  checked={options.requiredTags.some(
                    (t) => t.toLowerCase() === tag.name.toLowerCase(),
                  )}
                  onChange={() => toggleTag(tag.name)}
                />
                <span>{tag.name}</span>
                <span className="muted count">{tag.count}</span>
              </label>
            ))}
            {options.requiredTags.length > 0 && (
              <button
                type="button"
                className="ghost small full"
                onClick={() => {
                  onChange({ ...options, requiredTags: [] });
                  close();
                }}
              >
                清除标签筛选
              </button>
            )}
          </div>
        )}
      </Dropdown>

      <Dropdown label="语言" count={options.languages.length}>
        {() => (
          <div className="dropdown-list">
            {languages.length === 0 && <div className="muted pad">还没有语言信息</div>}
            {languages.map((lang) => (
              <label key={lang.code} className="dropdown-item">
                <input
                  type="checkbox"
                  checked={options.languages.includes(lang.code)}
                  onChange={() => toggleLanguage(lang.code)}
                />
                <span>{lang.label}</span>
                <span className="muted count">{lang.count}</span>
              </label>
            ))}
          </div>
        )}
      </Dropdown>

      <div className="sortgroup">
        <select
          value={options.sortField}
          disabled={disabled}
          onChange={(e) => onChange({ ...options, sortField: e.target.value as SortField })}
          aria-label="排序字段"
        >
          {(Object.keys(SORT_LABELS) as SortField[]).map((field) => (
            <option key={field} value={field}>
              {SORT_LABELS[field]}
            </option>
          ))}
        </select>
        <button
          type="button"
          className="ghost"
          disabled={disabled}
          title={options.sortDirection === 'desc' ? '当前降序，点击切换升序' : '当前升序，点击切换降序'}
          onClick={() =>
            onChange({
              ...options,
              sortDirection: options.sortDirection === 'desc' ? 'asc' : 'desc',
            })
          }
        >
          {options.sortDirection === 'desc' ? '↓ 降序' : '↑ 升序'}
        </button>
      </div>

      {/*
        A select rather than a checkbox: there are three modes now (by translation, by series, or
        flat), and a checkbox cannot express three states. The setting lives in the settings panel
        too; this is the quick toggle for when the current view is not the one wanted.
      */}
      <select
        value={options.groupMode}
        disabled={disabled}
        aria-label="分组方式"
        title="按关联分组的方式；也可以在设置里改"
        onChange={(e) => onChange({ ...options, groupMode: e.target.value as GroupMode })}
      >
        <option value="translation">按译本分组</option>
        <option value="series">按系列分组</option>
        <option value="none">不分组</option>
      </select>

      <label className="checkbox" title="只记录、尚未下载文件的条目">
        <input
          type="checkbox"
          checked={options.includeRecordOnly}
          disabled={disabled}
          onChange={(e) => onChange({ ...options, includeRecordOnly: e.target.checked })}
        />
        <span>含仅记录</span>
      </label>

      {filtersActive && (
        <button
          type="button"
          className="ghost small"
          disabled={disabled}
          onClick={() => {
            setSearchDraft('');
            onChange({
              ...options,
              search: '',
              requiredTags: [],
              languages: [],
              includeRecordOnly: true,
            });
          }}
        >
          重置
        </button>
      )}
</div>
  );
}

interface DropdownProps {
  label: string;
  count: number;
  children: (close: () => void) => React.ReactNode;
}

/**
 * A labelled button that opens a panel.
 *
 * The panel is a CSS popover rather than an absolutely positioned box. A panel opened from the
 * toolbar hangs below the bar, and every attempt to clip the bar for tidiness also clipped the
 * panel — one ancestor at a time. The top layer is browsed above all of that, and gives light
 * dismissal and Escape handling for free.
 */
function Dropdown({ label, count, children }: DropdownProps) {
  const [open, setOpen] = useState(false);
  const buttonRef = useRef<HTMLButtonElement>(null);
  const panelRef = useRef<HTMLDivElement>(null);

  /** Places the panel under its trigger, in viewport coordinates. */
  const position = useCallback(() => {
    const button = buttonRef.current;
    const panel = panelRef.current;
    if (button === null || panel === null) return;
    const rect = button.getBoundingClientRect();
    panel.style.left = `${Math.round(rect.left)}px`;
    panel.style.top = `${Math.round(rect.bottom + 4)}px`;
  }, []);

  // Kept in step with the popover's own state, including light dismissal and Escape, which the
  // browser performs without telling React.
  useEffect(() => {
    const panel = panelRef.current;
    if (panel === null) return;
    const sync = (): void => setOpen(panel.matches(':popover-open'));
    panel.addEventListener('toggle', sync);
    return () => panel.removeEventListener('toggle', sync);
  }, []);

  // Reposition while open, so it follows a window resize or a scroll.
  useEffect(() => {
    if (!open) return;
    position();
    window.addEventListener('resize', position);
    window.addEventListener('scroll', position, true);
    return () => {
      window.removeEventListener('resize', position);
      window.removeEventListener('scroll', position, true);
    };
  }, [open, position]);

  return (
    <div className="dropdown">
      <button
        ref={buttonRef}
        type="button"
        className={count > 0 ? 'active' : ''}
        aria-expanded={open}
        onClick={() => {
          const panel = panelRef.current;
          if (panel === null) return;
          if (panel.matches(':popover-open')) {
            panel.hidePopover();
          } else {
            position();
            panel.showPopover();
          }
        }}
      >
        {label}
        {count > 0 && <span className="badge">{count}</span>}
        <span className="caret">▾</span>
      </button>
      <div className="dropdown-panel" popover="auto" ref={panelRef}>
        {children(() => panelRef.current?.hidePopover())}
      </div>
    </div>
  );
}
