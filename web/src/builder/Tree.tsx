import { Icon, Number } from '../parts/index.ts';
import { samePath, type Block, type Path, type Slot } from './draft.ts';
import { blockWords } from './words.ts';

export type Problem = { path: Path; message: string; fix: string };

type TreeProps = {
  root: Block;
  /** What a person calls each ticker, where the server has a name for it. */
  names: Map<string, string>;
  problems: Problem[];
  /** Open the editor of the block at a path. */
  onEdit: (path: Path) => void;
  /** Add to one list of the block at a path. */
  onAdd: (path: Path, slot: Slot) => void;
};

const KIND_CLASS: Record<Block['block'], string> = { asset: 'asset', weight: 'split', if: 'if', filter: 'pick' };

function Rule({ block, path, share, tree }: { block: Block; path: Path; share?: number; tree: TreeProps }) {
  const problems = tree.problems.filter((problem) => samePath(problem.path, path));
  const lists: { slot: Slot; blocks: Block[] }[] =
    block.block === 'if'
      ? [
          { slot: 'then', blocks: block.then },
          { slot: 'else', blocks: block.else },
        ]
      : block.block === 'asset'
        ? []
        : [{ slot: 'children', blocks: block.children }];
  const shares = block.block === 'weight' && block.weighting.type === 'specified' ? block.weighting.shares : null;

  return (
    <li className="rule">
      <button type="button" className={`pill pill-${KIND_CLASS[block.block]}${problems.length ? ' pill-problem' : ''}`} onClick={() => tree.onEdit(path)}>
        {block.block === 'asset' ? (
          <>
            <span className="ticker">{blockWords(block)}</span>
            {'symbol' in block.asset && tree.names.has(block.asset.symbol) && <span className="pill-name">{tree.names.get(block.asset.symbol)}</span>}
            {share !== undefined && <Number kind="percent" value={share} />}
          </>
        ) : (
          blockWords(block)
        )}
        <span className="sr-only">, edit</span>
      </button>
      {problems.map((problem) => (
        <p key={problem.message} className="rule-problem">
          <Icon name="alert" size="sm" />
          {`${problem.message} ${problem.fix}`}
        </p>
      ))}
      {lists.map(({ slot, blocks }) => (
        <div key={slot} className="rule-list">
          {slot === 'else' && <span className="pill pill-if pill-label">Otherwise</span>}
          <ul className="rules" role="list">
            {blocks.map((child, place) => (
              <Rule key={place} block={child} path={[...path, slot, place]} share={shares?.[place]} tree={tree} />
            ))}
            <li className="rule">
              <button type="button" className="slot" onClick={() => tree.onAdd(path, slot)}>
                <Icon name="plus" size="sm" />
                Add
              </button>
            </li>
          </ul>
        </div>
      ))}
    </li>
  );
}

/** The rules as pills, each list set in under the rule it belongs to. Every pill opens its editor. */
export function Tree(props: TreeProps) {
  return (
    <ul className="rules rules-root" role="list">
      <Rule block={props.root} path={[]} tree={props} />
    </ul>
  );
}
