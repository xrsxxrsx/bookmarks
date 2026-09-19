import { useState } from 'react';

interface Props {
  onCreate: (title: string, author: string | null) => Promise<void>;
  onClose: () => void;
}

/**
 * Records a work that has no file yet.
 *
 * The point is that the library should not depend on having downloaded something. AO3 hides and
 * deletes works, and a bookmark whose file was never saved is exactly what this application is for
 * — so an entry has to be creatable by hand rather than only appearing as a side effect of an
 * import. It is stored as `record_only` and shown with a hollow status dot.
 */
export function CreateWorkDialog({ onCreate, onClose }: Props) {
  const [title, setTitle] = useState('');
  const [author, setAuthor] = useState('');
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);

  async function submit(): Promise<void> {
    if (title.trim() === '') {
      setError('标题不能为空');
      return;
    }
    setBusy(true);
    setError(null);
    try {
      await onCreate(title.trim(), author.trim() === '' ? null : author.trim());
      onClose();
    } catch (e: unknown) {
      setError(e instanceof Error ? e.message : String(e));
    } finally {
      setBusy(false);
    }
  }

  return (
    <div className="modal-backdrop" onClick={onClose}>
      <div className="modal" onClick={(e) => e.stopPropagation()}>
        <h2>新建条目</h2>
        <p className="muted">
          只记录信息，不包含文件 —— 用于先存档、以后再下载，或者只有链接的情况。
          建好后可以在右侧补上作者、Summary、标签、语言和字数，也可以随时导入文件挂上去。
        </p>

        <div className="field">
          <span>标题</span>
          <input
            autoFocus
            value={title}
            placeholder="必填"
            onChange={(e) => setTitle(e.target.value)}
            onKeyDown={(e) => {
              if (e.key === 'Enter') void submit();
            }}
          />
        </div>
        <div className="field">
          <span>作者</span>
          <input
            value={author}
            placeholder="可以留空"
            onChange={(e) => setAuthor(e.target.value)}
            onKeyDown={(e) => {
              if (e.key === 'Enter') void submit();
            }}
          />
        </div>

        {error !== null && <div className="inline-error">{error}</div>}

        <div className="modal-actions">
          <button type="button" disabled={busy || title.trim() === ''} onClick={() => void submit()}>
            {busy ? '正在创建…' : '创建'}
          </button>
          <button type="button" className="ghost" disabled={busy} onClick={onClose}>
            取消
          </button>
        </div>
      </div>
    </div>
  );
}
