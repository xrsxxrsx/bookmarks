import { useEffect, useState } from 'react';

import * as api from '../api';
import type { Settings, Startup } from '../types';

interface Props {
  settings: Settings;
  onSave: (patch: Partial<Settings>) => Promise<void>;
  /** Adopts the result of a library move or switch, which the backend already performed. */
  onLibraryChanged: (info: Startup) => Promise<void>;
  onClose: () => void;
}

/**
 * Everything configurable, in one place.
 *
 * The library location is deliberately the most prominent item: it is a decision about where
 * every file goes, so it should be findable rather than buried. Changing it can either move the
 * existing library or point at a different one, and those are different enough to be separate
 * buttons — a single "change" button would force a choice the user has not been asked to make.
 */
export function SettingsPanel({ settings, onSave, onLibraryChanged, onClose }: Props) {
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [notice, setNotice] = useState<string | null>(null);
  const [pendingPath, setPendingPath] = useState<string | null>(null);
  const [dismissed, setDismissed] = useState<
    { external_id: string; title: string | null; author: string | null }[]
  >([]);

  // Loaded on open rather than kept in sync, because this list only changes from here.
  useEffect(() => {
    let cancelled = false;
    api
      .listDismissedPending()
      .then((list) => {
        if (!cancelled) setDismissed(list);
      })
      .catch(() => {
        // Not being able to list them is not worth an error; the section just stays empty.
      });
    return () => {
      cancelled = true;
    };
  }, []);

  async function restore(externalId: string): Promise<void> {
    await api.restorePending(externalId);
    setDismissed((prev) => prev.filter((d) => d.external_id !== externalId));
    setNotice('已恢复提示，下次回到书库时会重新出现。');
  }

  async function run(action: () => Promise<void>): Promise<void> {
    setBusy(true);
    setError(null);
    try {
      await action();
    } catch (e: unknown) {
      setError(e instanceof Error ? e.message : String(e));
    } finally {
      setBusy(false);
    }
  }

  async function pickTarget(): Promise<void> {
    await run(async () => {
      const dir = await api.pickLibraryFolder('选择书库目录');
      if (dir !== null) {
        setPendingPath(dir);
        setNotice(null);
      }
    });
  }

  return (
    <div className="modal-backdrop" onClick={onClose}>
      <div className="modal wide" onClick={(e) => e.stopPropagation()}>
        <h2>设置</h2>

        <div className="settings-body">
          <section className="settings-section">
            <h3>外观</h3>
            <div className="settings-row">
              <span>主题</span>
              <div className="settings-control">
                <select
                  value={settings.theme}
                  disabled={busy}
                  onChange={(e) => void run(() => onSave({ theme: e.target.value }))}
                >
                  <option value="light">浅色</option>
                  <option value="dark">深色</option>
                </select>
              </div>
            </div>
            <div className="settings-row">
              <span>界面字号</span>
              <div className="settings-control">
                {settings.font_size_choices.map((px) => (
                  <button
                    key={px}
                    type="button"
                    className={settings.font_size_px === px ? '' : 'ghost'}
                    disabled={busy}
                    onClick={() => void run(() => onSave({ font_size_px: px }))}
                  >
                    {px}px
                  </button>
                ))}
              </div>
            </div>
          </section>

          <section className="settings-section">
            <h3>行为</h3>
            <div className="settings-row">
              <span>分组方式</span>
              <div className="settings-control">
                <select
                  value={settings.group_mode}
                  disabled={busy}
                  onChange={(e) => void run(() => onSave({ group_mode: e.target.value }))}
                >
                  <option value="translation">按译本分组</option>
                  <option value="series">按系列分组</option>
                  <option value="none">不分组</option>
                </select>
              </div>
            </div>
            <div className="settings-row">
              <span>
                同格式重复导入
                <br />
                <span className="muted">导入时还可以逐个覆盖这个默认</span>
              </span>
              <div className="settings-control">
                <select
                  value={settings.keep_both_versions ? 'keep' : 'replace'}
                  disabled={busy}
                  onChange={(e) =>
                    void run(() => onSave({ keep_both_versions: e.target.value === 'keep' }))
                  }
                >
                  <option value="keep">保留两份（旧版归档）</option>
                  <option value="replace">只留新的（替换）</option>
                </select>
              </div>
            </div>
          </section>

          <section className="settings-section">
            <h3>已忽略的译本提示</h3>
            {dismissed.length === 0 ? (
              <div className="settings-row">
                <span />
                <div className="settings-control muted">
                  没有已忽略的提示。书库页上那些「已记录但尚未导入」的译本，可以按「这些不需要」清掉。
                </div>
              </div>
            ) : (
              <div className="settings-list">
                {dismissed.map((d) => (
                  <div className="settings-row" key={d.external_id}>
                    <span className="muted">works/{d.external_id}</span>
                    <div className="settings-control">
                      <span className="path-value">{d.title ?? '(无标题)'}</span>
                      {d.author !== null && <span className="muted">by {d.author}</span>}
                      <button
                        type="button"
                        className="ghost small"
                        disabled={busy}
                        onClick={() => void run(() => restore(d.external_id))}
                      >
                        恢复提示
                      </button>
                    </div>
                  </div>
                ))}
              </div>
            )}
          </section>

          <section className="settings-section">
            <h3>书库位置</h3>
            <div className="settings-row">
              <span>当前位置</span>
              <div className="settings-control path-value" title={settings.library_path}>
                {settings.library_path}
              </div>
            </div>
            <div className="settings-row">
              <span />
              <div className="settings-control">
                <button type="button" className="ghost small" onClick={() => void api.openLibraryFolder()}>
                  打开目录
                </button>
                <button
                  type="button"
                  className="ghost small"
                  onClick={() => void api.openSettingsFolder()}
                >
                  打开设置文件所在目录
                </button>
              </div>
            </div>

            <div className="settings-row">
              <span>更改位置</span>
              <div className="settings-control">
                <button type="button" disabled={busy} onClick={() => void pickTarget()}>
                  选择目录…
                </button>
                {pendingPath !== null && (
                  <>
                    <div className="path-value" title={pendingPath}>
                      {pendingPath}
                    </div>
                    <button
                      type="button"
                      disabled={busy}
                      onClick={() =>
                        void run(async () => {
                          await onLibraryChanged(await api.setLibraryPath(pendingPath, true));
                          setPendingPath(null);
                          setNotice('书库已移动到新位置。');
                        })
                      }
                    >
                      移动到这里
                    </button>
                    <button
                      type="button"
                      className="ghost"
                      disabled={busy}
                      onClick={() =>
                        void run(async () => {
                          await onLibraryChanged(await api.setLibraryPath(pendingPath, false));
                          setPendingPath(null);
                          setNotice('已切换到这个位置。');
                        })
                      }
                    >
                      用这里的现有书库
                    </button>
                  </>
                )}
              </div>
            </div>

            <div className="settings-hint muted">
              「移动到这里」会把当前书库整体搬过去（先复制校验、成功后才删除原位置）。
              「用这里的现有书库」不改动任何文件，只是切换过去。
              <br />
              目标目录如果不为空且不是书库，会被拒绝 —— 这是为了防止覆盖别的数据。
              <br />
              设置文件本身固定在 <code>{settings.settings_file}</code>，只有几百字节；书库可以放在任何盘。
            </div>
          </section>
        </div>

        {error !== null && <div className="inline-error">{error}</div>}
        {notice !== null && <div className="settings-notice">{notice}</div>}

        <div className="modal-actions">
          <button type="button" onClick={onClose} disabled={busy}>
            关闭
          </button>
        </div>
      </div>
    </div>
  );
}
