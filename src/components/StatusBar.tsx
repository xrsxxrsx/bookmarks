import type { Counts } from '../types';

interface Props {
  counts: Counts;
  /** How many rows the current filter and search produce. */
  showing: number;
  root: string;
}

export function StatusBar({ counts, showing, root }: Props) {
  const filtered = showing !== counts.works_total;

  return (
    <footer className="statusbar">
      <span className="stat" title="作品数按「篇」计，同一篇文的多个格式算一篇">
        {filtered ? (
          <>
            筛选出 <b>{showing}</b> / {counts.works_total} 篇
          </>
        ) : (
          <>
            共 <b>{counts.works_total}</b> 篇
          </>
        )}
      </span>
      <span className="sep">·</span>
      <span className="stat">{counts.works_with_file} 篇有文件</span>
      {counts.works_record_only > 0 && (
        <>
          <span className="sep">·</span>
          <span className="stat" title="只有元数据、还没有下载文件的条目">
            {counts.works_record_only} 篇仅记录
          </span>
        </>
      )}
      <span className="sep">·</span>
      <span className="stat" title="磁盘上的文件数量，一个作品可能有多个格式">
        {counts.files_total} 个文件
      </span>
      {counts.works_related > 0 && (
        <>
          <span className="sep">·</span>
          <span className="stat">{counts.works_related} 篇有关联版本</span>
        </>
      )}
      {counts.pending_relations > 0 && (
        <>
          <span className="sep">·</span>
          <span className="stat" title="已记录、但对应文件还没导入的译本">
            {counts.pending_relations} 个译本待导入
          </span>
        </>
      )}
      <span className="spacer" />
      <span className="stat path" title={root}>
        书库：{root}
      </span>
    </footer>
  );
}
