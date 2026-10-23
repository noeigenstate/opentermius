import { useEffect, useId, useRef, type ReactNode } from "react";

export function Dialog({ title, children, onClose, busy = false }: {
  title: string;
  children: ReactNode;
  onClose: () => void;
  busy?: boolean;
}) {
  const dialog = useRef<HTMLDialogElement>(null);
  const titleId = useId();
  useEffect(() => {
    const element = dialog.current!;
    element.showModal();
    return () => { element.close(); };
  }, []);
  return (
    <dialog ref={dialog} className="dialog" aria-labelledby={titleId} onCancel={(event) => {
      event.preventDefault();
      if (!busy) onClose();
    }}>
      <h2 id={titleId}>{title}</h2>
      {children}
    </dialog>
  );
}
