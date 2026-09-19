import { useCallback, useEffect, useMemo, useState } from 'react';

import * as api from './api';
import { WorkTable } from './components/WorkTable';
import { WorkDetail } from './components/WorkDetail';
import { FilterBar } from './components/FilterBar';
import { StatusBar } from './components/StatusBar';
import { ImportDialog } from './components/ImportDialog';
import { CreateWorkDialog } from './components/CreateWorkDialog';
import { SettingsPanel } from './components/SettingsPanel';
import { buildRows, defaultViewOptions, tagFacets, type ViewOptions } from './library';
import { applyFontScale, type FontScale } from './fontScale';
import type { ImportPreview, Library, Settings, Work } from './types';

/** The four states the window can be in before a library is usable. */
type Boot =
  | { phase: 'loading' }
  | { phase: 'ready'; library: Library; settings: Settings }
  | { phase: 'unconfigured'; settings: Settings }
  | { phase: 'error'; message: string; settings: Settings };

type ImportState =
  | { phase: 'idle' }
  | { phase: 'analyzing' }
  | { phase: 'preview'; preview: ImportPreview }
  | { phase: 'applying'; preview: ImportPreview }
  | { phase: 'done'; message: string };

/** Maps the stored font size onto the scale name the stylesheet uses. */
function scaleFromPx(px: number): FontScale {
  if (px <= 13) return 'small';
  if (px >= 18) return 'huge';
  if (px >= 16) return 'large';
  return 'normal';
}

export default function App() {
  const [boot, setBoot] = useState<Boot>({ phase: 'loading' });
  const [actionError, setActionError] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);
  const [options, setOptions] = useState<ViewOptions>(defaultViewOptions);
  const [selectedId, setSelectedId] = useState<number | null>(null);
  const [importState, setImportState] = useState<ImportState>({ phase: 'idle' });
  const [fontScale, setFontScale] = useState<FontScale>('normal');
  const [settingsOpen, setSettingsOpen] = useState(false);
  const [createOpen, setCreateOpen] = useState(false);

  const settings = boot.phase === 'loading' ? null : boot.settings;
  const library = boot.phase === 'ready' ? boot.library : null;

  // The theme is applied as a class on the root so the stylesheet can switch palettes, and the
  // font size drives the root font — everything else is in rem, so it scales together.
  useEffect(() => {
    const theme = settings?.theme ?? 'light';
    document.documentElement.dataset.theme = theme;
  }, [settings?.theme]);

  // The stored grouping mode seeds the view. After that the toolbar's own control owns it, so
  // this only fires when the saved setting changes.
  useEffect(() => {
    const mode = settings?.group_mode;
    if (mode === 'translation' || mode === 'series' || mode === 'none') {
      setOptions((prev) => (prev.groupMode === mode ? prev : { ...prev, groupMode: mode }));
    }
  }, [settings?.group_mode]);

  useEffect(() => {
    applyFontScale(fontScale);
  }, [fontScale]);

  /** Loads the whole library and records it, or explains why it could not be loaded. */
  const load = useCallback(async (): Promise<void> => {
    const info = await api.startup();
    setFontScale(scaleFromPx(info.settings.font_size_px));
    if (info.state === 'unconfigured') {
      setBoot({ phase: 'unconfigured', settings: info.settings });
      return;
    }
    try {
      const lib = await api.loadLibrary();
      setBoot({ phase: 'ready', library: lib, settings: info.settings });
    } catch (e: unknown) {
      setBoot({
        phase: 'error',
        message: `${info.library_path}：${e instanceof Error ? e.message : String(e)}`,
        settings: info.settings,
      });
    }
  }, []);

  useEffect(() => {
    let cancelled = false;
    load().catch((e: unknown) => {
      if (!cancelled) {
        setBoot({
          phase: 'error',
          message: e instanceof Error ? e.message : String(e),
          settings: {
            library_path: '',
            default_library_path: '',
            settings_file: '',
            theme: 'light',
            font_size_px: 14,
            group_mode: 'translation',
            keep_both_versions: true,
            font_size_choices: [13, 14, 16, 18],
          },
        });
      }
    });
    return () => {
      cancelled = true;
    };
  }, [load]);

  async function refresh(): Promise<void> {
    const next = await api.loadLibrary();
    setBoot((prev) => (prev.phase === 'ready' ? { ...prev, library: next } : prev));
  }

  /** Saves settings and reflects them locally, so the change is visible immediately. */
  async function updateSettings(patch: Partial<Settings>): Promise<void> {
    const saved = await api.saveSettings(patch);
    if (patch.font_size_px !== undefined) setFontScale(scaleFromPx(saved.font_size_px));
    setBoot((prev) => (prev.phase === 'loading' ? prev : { ...prev, settings: saved }));
  }

  /** Applies the startup result of a library change, which the backend already performed. */
  async function adoptLibraryChange(info: Awaited<ReturnType<typeof api.setLibraryPath>>) {
    setFontScale(scaleFromPx(info.settings.font_size_px));
    if (info.state === 'unconfigured') {
      setBoot({ phase: 'unconfigured', settings: info.settings });
      return;
    }
    try {
      const lib = await api.loadLibrary();
      setBoot({ phase: 'ready', library: lib, settings: info.settings });
    } catch (e: unknown) {
      setBoot({
        phase: 'error',
        message: `${info.library_path}：${e instanceof Error ? e.message : String(e)}`,
        settings: info.settings,
      });
    }
  }

  /** Runs an action, surfacing any failure instead of leaving the UI silent. */
  async function guarded(action: () => Promise<void>): Promise<void> {
    setActionError(null);
    setBusy(true);
    try {
      await action();
    } catch (e: unknown) {
      setActionError(e instanceof Error ? e.message : String(e));
    } finally {
      setBusy(false);
    }
  }

  async function startImport(pick: () => Promise<string[]>): Promise<void> {
    await guarded(async () => {
      const paths = await pick();
      if (paths.length === 0) return;
      setImportState({ phase: 'analyzing' });
      try {
        const preview = await api.analyzeImport(paths);
        setImportState({ phase: 'preview', preview });
      } catch (e) {
        setImportState({ phase: 'idle' });
        throw e;
      }
    });
  }

  async function confirmImport(token: string, keepBoth: boolean): Promise<void> {
    await guarded(async () => {
      const preview =
        importState.phase === 'preview' || importState.phase === 'applying'
          ? importState.preview
          : null;
      if (preview !== null) setImportState({ phase: 'applying', preview });
      const report = await api.executeImport(token, keepBoth);
      await refresh();
      const parts = [
        `新建 ${report.works_created.length} 篇`,
        report.works_matched.length > 0 ? `并入 ${report.works_matched.length} 篇已有作品` : null,
        report.files_stored > 0 ? `存储 ${report.files_stored} 个文件` : null,
        report.files_skipped > 0 ? `跳过 ${report.files_skipped} 个重复文件` : null,
        report.relations_stored > 0 ? `关联 ${report.relations_stored} 对译本` : null,
      ].filter((p): p is string => p !== null);
      setImportState({ phase: 'done', message: parts.join('，') || '没有需要写入的内容' });
    });
  }

  /** Reassigns a preview work's files, replacing the preview with the rebuilt plan. */
  async function assignImportTarget(
    token: string,
    key: string,
    workId: number | null,
  ): Promise<void> {
    await guarded(async () => {
      const preview = await api.assignImportTarget(token, key, workId);
      setImportState({ phase: 'preview', preview });
    });
  }

  async function cancelImport(token: string): Promise<void> {
    setImportState({ phase: 'idle' });
    try {
      await api.discardImport(token);
    } catch {
      // A preview that cannot be discarded has already expired, which is harmless.
    }
  }

  async function saveWork(update: Parameters<typeof api.updateWork>[0]): Promise<void> {
    await guarded(async () => {
      const updated = await api.updateWork(update);
      setBoot((prev) =>
        prev.phase === 'ready'
          ? {
              ...prev,
              library: {
                ...prev.library,
                works: prev.library.works.map((w) => (w.id === updated.id ? updated : w)),
              },
            }
          : prev,
      );
    });
  }

  async function copyAo3Tags(workId: number): Promise<void> {
    await guarded(async () => {
      const updated = await api.importAo3Tags(workId);
      setBoot((prev) =>
        prev.phase === 'ready'
          ? {
              ...prev,
              library: {
                ...prev.library,
                works: prev.library.works.map((w) => (w.id === updated.id ? updated : w)),
              },
            }
          : prev,
      );
    });
  }

  /** Replaces a work's series membership. */
  async function saveSeries(
    workId: number,
    series: { name: string; position: number | null }[],
  ): Promise<void> {
    await guarded(async () => {
      const updated = await api.setWorkSeries(workId, series);
      setBoot((prev) =>
        prev.phase === 'ready'
          ? {
              ...prev,
              library: {
                ...prev.library,
                works: prev.library.works.map((w) => (w.id === updated.id ? updated : w)),
                // The series list feeds the editor's suggestions, so it has to follow.
                all_series: prev.library.all_series.includes(series[0]?.name ?? '')
                  ? prev.library.all_series
                  : [
                      ...prev.library.all_series,
                      ...series.map((s) => s.name).filter((n) => !prev.library.all_series.includes(n)),
                    ].sort((a, b) => a.localeCompare(b)),
              },
            }
          : prev,
      );
    });
  }

  /**
   * Deletes a work and everything under it.
   *
   * The whole library is reloaded rather than the work being filtered out locally: deletion also
   * removes files, prunes now-unused tags and series, and can leave dangling relations, so
   * re-reading is both simpler and more truthful.
   */
  async function deleteWork(workId: number): Promise<void> {
    await guarded(async () => {
      const removed = await api.deleteWork(workId);
      setSelectedId(null);
      await refresh();
      setImportState({
        phase: 'done',
        message: `已删除该作品${removed > 0 ? `及其 ${removed} 个文件` : ''}`,
      });
    });
  }

  const rows = useMemo(
    () => (library === null ? [] : buildRows(library.works, options)),
    [library, options],
  );

  // Derived from the works, not from the snapshot the backend returned at load time, so a
  // tag added in the edit panel shows up in the filter immediately.
  const tagOptions = useMemo(
    () => (library === null ? [] : tagFacets(library.works)),
    [library],
  );

  /** Clicking a tag in a row filters by it; clicking an active one clears it. */
  function toggleTagFilter(tag: string): void {
    setOptions((prev) => {
      const has = prev.requiredTags.some((t) => t.toLowerCase() === tag.toLowerCase());
      return {
        ...prev,
        requiredTags: has
          ? prev.requiredTags.filter((t) => t.toLowerCase() !== tag.toLowerCase())
          : [...prev.requiredTags, tag],
      };
    });
  }

  const selected: Work | null = useMemo(() => {
    if (library === null || selectedId === null) return null;
    return library.works.find((w) => w.id === selectedId) ?? null;
  }, [library, selectedId]);

  if (boot.phase === 'loading') {
    return <div className="loading">正在载入书库…</div>;
  }

  if (boot.phase === 'unconfigured') {
    return (
      <div className="fatal">
        <h1>还没有指定书库位置</h1>
        <p className="muted">
          书库（数据库 + 所有文件）会放在下面这个目录里。也可以换到你自己的位置。
        </p>
        <pre>{boot.settings.default_library_path}</pre>
        <div className="setup-actions">
          <button
            type="button"
            onClick={() =>
              void guarded(async () => {
                await adoptLibraryChange(await api.useDefaultLibrary(false));
              })
            }
          >
            用这个位置
          </button>
          <button
            type="button"
            className="ghost"
            onClick={() =>
              void guarded(async () => {
                const dir = await api.pickLibraryFolder('选择书库目录');
                if (dir === null) return;
                await adoptLibraryChange(await api.setLibraryPath(dir, false));
              })
            }
          >
            选择其他目录…
          </button>
        </div>
      </div>
    );
  }

  if (boot.phase === 'error') {
    return (
      <div className="fatal">
        <h1>无法打开书库</h1>
        <pre>{boot.message}</pre>
        <p className="muted">
          当前设置的位置是 <code>{boot.settings.library_path}</code>。
          如果这个目录被删掉或移动了，可以重新指定；否则检查一下它是否可读写。
        </p>
        <div className="setup-actions">
          <button
            type="button"
            onClick={() =>
              void guarded(async () => {
                const dir = await api.pickLibraryFolder('选择书库目录');
                if (dir === null) return;
                await adoptLibraryChange(await api.setLibraryPath(dir, false));
              })
            }
          >
            重新指定书库目录…
          </button>
          <button type="button" className="ghost" onClick={() => void api.openSettingsFolder()}>
            打开设置文件所在目录
          </button>
        </div>
      </div>
    );
  }

  // Narrowed here rather than through the derived `library` const: a const computed before the
  // phase checks is not narrowed by them, so the library is taken straight off `boot`.
  const openLibrary = boot.phase === 'ready' ? boot.library : null;
  if (openLibrary === null) {
    return <div className="loading">正在载入书库…</div>;
  }

  return (
    <div className="app">
      <header className="topbar">
        <div className="brand">bookmarks</div>
        <FilterBar
          options={options}
          onChange={setOptions}
          tags={tagOptions}
          works={openLibrary.works}
          disabled={busy}
        />
        <div className="topbar-actions">
          <button type="button" onClick={() => void startImport(api.pickFiles)} disabled={busy}>
            导入文件
          </button>
          <button type="button" onClick={() => void startImport(api.pickFolder)} disabled={busy}>
            导入文件夹
          </button>
          <button
            type="button"
            className="ghost"
            disabled={busy}
            title="只记录信息、不含文件"
            onClick={() => setCreateOpen(true)}
          >
            新建条目
          </button>
          <button type="button" className="ghost" onClick={() => void api.openLibraryFolder()}>
            打开书库目录
          </button>
          <button
            type="button"
            className="ghost"
            title="设置"
            onClick={() => setSettingsOpen(true)}
          >
            设置
          </button>
        </div>
      </header>

      {createOpen && (
        <CreateWorkDialog
          onClose={() => setCreateOpen(false)}
          onCreate={async (title, author) =>
            guarded(async () => {
              const created = await api.createWork(title, author);
              await refresh();
              setSelectedId(created.id);
              setImportState({ phase: 'done', message: `已新建条目「${created.title}」` });
            })
          }
        />
      )}

      {settingsOpen && (
        <SettingsPanel
          settings={boot.settings}
          onSave={updateSettings}
          onLibraryChanged={adoptLibraryChange}
          onClose={() => setSettingsOpen(false)}
        />
      )}

      {actionError !== null && (
        <div className="banner error">
          <span>{actionError}</span>
          <button type="button" className="ghost small" onClick={() => setActionError(null)}>
            关闭
          </button>
        </div>
      )}

      {importState.phase === 'done' && (
        <div className="banner ok">
          <span>{importState.message}</span>
          <button type="button" className="ghost small" onClick={() => setImportState({ phase: 'idle' })}>
            关闭
          </button>
        </div>
      )}

      {openLibrary.pending.length > 0 && importState.phase === 'idle' && (
        <div className="banner info">
          <span>
            有 {openLibrary.pending.length} 个译本已记录但尚未导入：
            {openLibrary.pending
              .slice(0, 3)
              .map((p) => p.title ?? `works/${p.external_id}`)
              .join('、')}
            {openLibrary.pending.length > 3 ? ' 等' : ''}
            。导入对应文件后会自动关联。
          </span>
          <button
            type="button"
            className="ghost small"
            disabled={busy}
            title="不再提示这些译本。记录会保留，随时可以在设置里恢复。"
            onClick={() =>
              void guarded(async () => {
                const cleared = await api.dismissAllPending();
                await refresh();
                setImportState({
                  phase: 'done',
                  message: `已忽略 ${cleared} 个译本提示，可在设置里恢复`,
                });
              })
            }
          >
            这些不需要，不再提示
          </button>
        </div>
      )}
      <main className="content">
        <WorkTable
          rows={rows}
          selectedId={selectedId}
          onSelect={setSelectedId}
          options={options}
          busy={busy}
          onTagClick={toggleTagFilter}
        />
        <WorkDetail
          work={selected}
          library={openLibrary}
          busy={busy}
          onSave={saveWork}
          onSaveSeries={saveSeries}
          onCopyAo3Tags={copyAo3Tags}
          onDelete={deleteWork}
          onSelectWork={setSelectedId}
        />
      </main>

      <StatusBar counts={openLibrary.counts} showing={rows.length} root={openLibrary.library_root} />

      {(importState.phase === 'preview' ||
        importState.phase === 'applying' ||
        importState.phase === 'analyzing') && (
        <ImportDialog
          state={importState}
          onConfirm={confirmImport}
          onCancel={cancelImport}
          onAssign={assignImportTarget}
        />
      )}
    </div>
  );
}
