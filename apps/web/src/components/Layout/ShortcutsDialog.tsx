import { Modal } from "@web/components/Dialog/Modal";
import { SHORTCUTS } from "@web/components/Layout/platform";

/** The keyboard shortcuts (Ctrl+/). */
export function ShortcutsDialog({ open, onClose }: { open: boolean; onClose: () => void }) {
  return (
    <Modal open={open} title="Keyboard Shortcuts" onClose={onClose}>
      <dl className="grid grid-cols-[auto_1fr] items-center gap-x-4 gap-y-2 text-sm">
        {SHORTCUTS.map((item) => (
          <div key={item.keys} className="contents">
            <dt>
              <kbd className="rounded-md border border-line bg-surface px-1.5 py-0.5 font-mono text-xs">
                {item.keys}
              </kbd>
            </dt>
            <dd>{item.action}</dd>
          </div>
        ))}
      </dl>
    </Modal>
  );
}
