import { useState } from 'react';

import type { FileAction, ImportPreview, PreviewWork } from '../types';

interface Props {
  state:
    | { phase: 'analyzing' }
    | { phase: 'preview'; preview: ImportPreview }
    | { phase: 'applying'; preview: ImportPreview };
  onConfirm: (token: string, keepBoth: boolean) => Promise<void>;
  onCancel: (token: string) => Promise<void>;
  /** Reassigns a preview work's files to an existing work, or back to a new one. */
  onAssign: (token: string, key: string, workId: number | null) => Promise<void>;
}

/** Describes what will happen to one file. */
function actionLabel(action: FileAction): { text: string; cls: string } {
  switch (action.kind) {
    case 'create_work':
      return { text: '新建作品', cls: 'ok' };
    case 'attach_to_existing':
      return { text: '挂到已有作品', cls: 'ok' };
    case 'replaces_stored_file':
      return { text: '替换已有文件', cls: 'warn' };
    case 'keep_both_versions':
      return { text: '保留两份（旧版归档）', cls: 'ok' };
    case 'skipped_duplicate':
      return { text: `跳过（已在 #${action.existing_work_id}）`, cls: 'muted-chip' };
  }
}

export function ImportDialog({ state, onConfirm, onCancel, onAssign }: Props) {
  const [busy, setBusy] = useState(false);
  // The version policy is a per-import decision, seeded from the setting and adjustable here
  // because the right answer differs by file.
  const [keepBoth, setKeepBoth] = useState<boolean | null>(null);

  if (state.phase === 'analyzing') {
    return (
      <div className="modal-backdrop">
        <div className="modal">
          <h2>正在分析文件…</h2>
          <p className="muted">读取元数据、计算哈希、判断归组。</p>
        </div>
      </div>
    );
  }

  const { preview } = state;
  const applying = state.phase === 'applying';
  const effectiveKeepBoth = keepBoth ?? preview.keep_both_versions;
  const nothingToDo = preview.summary.files_to_store === 0 && preview.summary.works_new === 0;
  const anyVersionChoice = preview.works.some((w) =>
    w.files.some((f) => f.needs_version_choice),
  );

  async function assign(key: string, workId: number | null): Promise<void> {
    setBusy(true);
    try {
      await onAssign(preview.token, key, workId);
    } finally {
      setBusy(false);
    }
  }

  return (
    <div className="modal-backdrop">
      <div className="modal wide">
        <h2>导入预览</h2>

        <div className="import-summary">
          <span>
            <b>{preview.summary.files_total}</b> 个文件
          </span>
          <span className="sep">·</span>
          <span>
            <b>{preview.summary.files_to_store}</b> 个将写入
          </span>
          {preview.summary.duplicates > 0 && (
            <>
              <span className="sep">·</span>
              <span className="muted">
                <b>{preview.summary.duplicates}</b> 个重复将被跳过
              </span>
            </>
          )}
          <span className="sep">·</span>
          <span>
            新建 <b>{preview.summary.works_new}</b> 篇
            {preview.summary.works_existing > 0 && (
              <>
                ，并入已有 <b>{preview.summary.works_existing}</b> 篇
              </>
            )}
          </span>
        </div>

        {/* Only shown when it can matter, so the dialog does not carry a decision that has no
            consequence for this batch. */}
        {anyVersionChoice && (
          <div className="import-option">
            <span className="muted">同格式重复时：</span>
            <label className="checkbox inline">
              <input
                type="radio"
                checked={effectiveKeepBoth}
                onChange={() => setKeepBoth(true)}
              />
              <span>保留两份</span>
            </label>
            <label className="checkbox inline">
              <input
                type="radio"
                checked={!effectiveKeepBoth}
                onChange={() => setKeepBoth(false)}
              />
              <span>只留新的（替换）</span>
            </label>
          </div>
        )}

        <div className="import-body">
          {preview.works.length === 0 && <p className="muted">没有可导入的文件。</p>}
          {preview.works.map((work) => (
            <WorkRow
              key={work.key}
              work={work}
              candidates={preview.candidates}
              disabled={busy || applying}
              onAssign={(workId) => void assign(work.key, workId)}
            />
          ))}
        </div>

        {(preview.pairs.length > 0 || preview.pending.length > 0) && (
          <div className="import-relations">
            {preview.pairs.map((pair) => (
              <div key={`${pair.from}-${pair.to}`}>
                将关联译本：works/{pair.from} ↔ works/{pair.to}
              </div>
            ))}
            {preview.pending.length > 0 && (
              <div className="muted">
                记录 {preview.pending.length} 个尚未导入的译本，等文件到来自动关联：
                {preview.pending.map((p) => p.title ?? `works/${p.external_id}`).join('、')}
              </div>
            )}
          </div>
        )}

        <div className="modal-actions">
          <button
            type="button"
            onClick={() => void onConfirm(preview.token, effectiveKeepBoth)}
            disabled={applying || busy || nothingToDo}
          >
            {applying ? '正在写入…' : '确认导入'}
          </button>
          <button
            type="button"
            className="ghost"
            onClick={() => void onCancel(preview.token)}
            disabled={applying || busy}
          >
            取消
          </button>
          {nothingToDo && <span className="muted">这些文件都已经在书库里了。</span>}
        </div>
      </div>
    </div>
  );
}

interface RowProps {
  work: PreviewWork;
  candidates: ImportPreview['candidates'];
  disabled: boolean;
  onAssign: (workId: number | null) => void;
}

function WorkRow({ work, candidates, disabled, onAssign }: RowProps) {
  const chosen = work.manual_target;
  const duplicatesOnly = work.all_duplicates;

  return (
    <div className="import-work">
      <div className="import-work-head">
        <span className={work.is_new ? 'chip ok' : 'chip subtle'}>
          {work.is_new ? '新建' : `并入 #${work.existing_work_id}`}
        </span>
        <span className="import-title">{work.title}</span>
        {work.author !== null && <span className="muted">by {work.author}</span>}
        {work.external_id !== null && <span className="muted">AO3 /{work.external_id}</span>}
        {work.language !== null && (
          <span className="lang-badge">{work.language.toUpperCase()}</span>
        )}
        {duplicatesOnly && <span className="chip muted-chip">内容已在库中</span>}
      </div>

      {/*
        The manual route. The automatic rules deliberately refuse to guess when a filename shares
        nothing with the stored title — which is exactly the case for a downloaded file — so
        without this the two copies could never be joined.

        Disabled when every file is a duplicate: the choice would be stored and shown, yet no file
        would move, which reads as a bug. Saying so is better than accepting a click that does
        nothing.
      */}
      <div className="import-target">
        <span className="muted">归入</span>
        <select
          value={chosen === null ? '' : String(chosen)}
          disabled={disabled || duplicatesOnly}
          title={
            duplicatesOnly
              ? '文件内容已经在书库里了，导入时会被跳过，所以无法改归入它处'
              : '选择这组文件要并入哪一篇已有作品'
          }
          onChange={(e) => onAssign(e.target.value === '' ? null : Number(e.target.value))}
        >
          <option value="">新建一篇作品</option>
          {candidates.map((c) => (
            <option key={c.id} value={c.id}>
              {/*
                The work id is not shown: it is an internal number the user has no use for, and it
                made every row start with the same-looking "#12". What matters is the title, the
                author, and which formats are already there.
              */}
              {c.title}
              {c.author === null ? '' : ` — ${c.author}`}
              {c.formats.length > 0 ? ` [${c.formats.join('/')}]` : ' [无文件]'}
            </option>
          ))}
        </select>
        {chosen !== null && (
          <span className="chip ok" title="这是你指定的，不是自动匹配的结果">
            手动指定
          </span>
        )}
        {duplicatesOnly && (
          <span className="muted">文件已存在，导入时会跳过这一组</span>
        )}
      </div>

      <ul className="import-files">
        {work.files.map((file) => {
          const label = actionLabel(file.action);
          return (
            <li key={file.source_path}>
              <span className="chip subtle">
                {file.format_class === 'other'
                  ? `Other · ${file.format_detail.toUpperCase()}`
                  : file.format_detail.toUpperCase()}
              </span>
              <span className="import-file-name" title={file.source_path}>
                {file.original_name}
              </span>
              {file.word_count !== null && (
                <span className="muted">
                  {file.word_count_is_estimated ? '~' : ''}
                  {file.word_count.toLocaleString('en-US')} 字
                </span>
              )}
              <span className={`chip ${label.cls}`}>{label.text}</span>
            </li>
          );
        })}
      </ul>
    </div>
  );
}
