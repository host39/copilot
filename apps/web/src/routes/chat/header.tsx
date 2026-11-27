import type { ChatResponse, ModelResponse, ProjectResponse } from "@copilot/contracts";
import { useMutation, useQueryClient } from "@tanstack/react-query";
import { describeError } from "@web/api/errors";
import { deleteChat, saveAsProject, saveInstructions, updateChat } from "@web/api/routes/chats";
import { keys } from "@web/api/routes/keys";
import { chatTitle } from "@web/components/Chats/ChatRow";
import { ConfirmDialog } from "@web/components/Dialog/ConfirmDialog";
import { Modal } from "@web/components/Dialog/Modal";
import { MenuButton } from "@web/components/Layout/Shell";
import { Menu, type MenuItem } from "@web/components/Menu/Menu";
import { ModelPicker } from "@web/components/ModelPicker/ModelPicker";
import { InstructionsForm, MoveForm, useChatSaved } from "@web/routes/chat/dialogs";
import { type MenuAction, menuActions, titleSchema } from "@web/routes/chat/settings";
import path, { link } from "@web/routes/path";
import { projectLabel } from "@web/routes/project/forms";
import { newScheduleLink } from "@web/routes/schedule/draft";
import { useRef, useState } from "react";
import {
  LuCalendarPlus,
  LuFolderInput,
  LuFolderOpen,
  LuFolderPlus,
  LuPin,
  LuPinOff,
  LuSave,
  LuScrollText,
  LuTrash2,
} from "react-icons/lu";
import { Link, useNavigate } from "react-router-dom";

/** The title; tap it to rename the chat (Enter or leaving the field saves, Escape cancels). */
function Title({ chat }: { chat: ChatResponse }) {
  const saved = useChatSaved();
  const [editing, setEditing] = useState(false);
  const [error, setError] = useState<string>();
  // Escape closes the field; the blur that follows must not save it.
  const cancelled = useRef(false);
  const rename = useMutation({
    mutationFn: (title: string) => updateChat(chat.id, { title }),
    onSuccess: (result) => {
      saved(result);
      setEditing(false);
    },
    onError: (failure) => setError(describeError(failure).message),
  });

  function submit(value: string) {
    if (cancelled.current || rename.isPending) return;
    const parsed = titleSchema.safeParse(value);
    if (!parsed.success) {
      setError(parsed.error.issues[0]?.message);
      return;
    }
    if (parsed.data === chat.title) {
      setEditing(false);
      return;
    }
    setError(undefined);
    rename.mutate(parsed.data);
  }

  if (!editing) {
    return (
      <h1 className="min-w-0 flex-1">
        <button
          type="button"
          onClick={() => {
            cancelled.current = false;
            setError(undefined);
            setEditing(true);
          }}
          title="Rename"
          className="max-w-full truncate rounded-lg px-2 py-1 text-left text-base font-semibold tracking-tight hover:bg-raised"
        >
          {chatTitle(chat)}
        </button>
      </h1>
    );
  }

  return (
    <form
      className="min-w-0 flex-1"
      onSubmit={(event) => {
        event.preventDefault();
        const value = new FormData(event.currentTarget).get("title");
        submit(typeof value === "string" ? value : "");
      }}
    >
      <label htmlFor="chat-title" className="sr-only">
        Chat title
      </label>
      <input
        id="chat-title"
        name="title"
        defaultValue={chat.title}
        // biome-ignore lint/a11y/noAutofocus: the field opens because the user asked to rename
        autoFocus
        disabled={rename.isPending}
        aria-invalid={error ? true : undefined}
        aria-describedby={error ? "chat-title-error" : undefined}
        onBlur={(event) => submit(event.currentTarget.value)}
        onKeyDown={(event) => {
          if (event.key === "Escape") {
            event.preventDefault();
            cancelled.current = true;
            setError(undefined);
            setEditing(false);
          }
        }}
        className="field-input mt-0 py-1.5"
      />
      {error ? (
        <p id="chat-title-error" role="alert" className="field-error">
          {error}
        </p>
      ) : null}
    </form>
  );
}

type Sheet = "instructions" | "move" | "delete" | "save-as-project" | "save-instructions" | null;

type ChatHeaderProps = {
  chat: ChatResponse;
  project: ProjectResponse | null;
  models: ModelResponse[];
  /** Tools are on for the next turn (some connections then turn reasoning off). */
  toolsOn: boolean;
};

/**
 * The chat's top bar: the drawer button on phones, the title (tap to rename), the model picker,
 * and the overflow menu (project, chat instructions, move to project, pin, schedule this prompt,
 * save as project or save instructions to project, delete).
 */
export function ChatHeader({ chat, project, models, toolsOn }: ChatHeaderProps) {
  const saved = useChatSaved();
  const client = useQueryClient();
  const navigate = useNavigate();
  const [sheet, setSheet] = useState<Sheet>(null);
  const close = () => setSheet(null);

  const change = useMutation({
    mutationFn: (body: Parameters<typeof updateChat>[1]) => updateChat(chat.id, body),
    onSuccess: (result) => saved(result),
  });
  const remove = useMutation({
    mutationFn: () => deleteChat(chat.id),
    onSuccess: () => {
      client.removeQueries({ queryKey: keys.chat(chat.id) });
      void client.invalidateQueries({ queryKey: [...keys.chats, "list"] });
      void client.invalidateQueries({ queryKey: keys.runs });
      navigate(path.home, { replace: true });
    },
  });
  const promote = useMutation({
    mutationFn: () => saveAsProject(chat.id),
    onSuccess: () => {
      void client.invalidateQueries({ queryKey: keys.chat(chat.id), exact: true });
      void client.invalidateQueries({ queryKey: [...keys.chats, "list"] });
      void client.invalidateQueries({ queryKey: keys.projects });
      close();
    },
  });

  const append = useMutation({
    mutationFn: () => saveInstructions(chat.id),
    onSuccess: () => {
      void client.invalidateQueries({ queryKey: keys.chat(chat.id), exact: true });
      void client.invalidateQueries({ queryKey: keys.projects });
      close();
    },
  });

  const items: Record<MenuAction, MenuItem> = {
    project: {
      key: "project",
      label: project ? `Open ${project.name}` : "Open project",
      icon: <LuFolderOpen />,
      onSelect: () => {
        if (chat.projectId) navigate(link.project(chat.projectId));
      },
    },
    instructions: {
      key: "instructions",
      label: "Chat instructions",
      icon: <LuScrollText />,
      onSelect: () => setSheet("instructions"),
    },
    move: {
      key: "move",
      label: "Move to project",
      icon: <LuFolderInput />,
      onSelect: () => setSheet("move"),
    },
    pin: {
      key: "pin",
      label: chat.pinned ? "Unpin" : "Pin",
      icon: chat.pinned ? <LuPinOff /> : <LuPin />,
      onSelect: () => change.mutate({ pinned: !chat.pinned }),
    },
    schedule: {
      key: "schedule",
      label: "Schedule this prompt...",
      icon: <LuCalendarPlus />,
      onSelect: () => navigate(newScheduleLink({ chatId: chat.id })),
    },
    "save-as-project": {
      key: "save-as-project",
      label: "Save as project",
      icon: <LuFolderPlus />,
      onSelect: () => {
        promote.reset();
        setSheet("save-as-project");
      },
    },
    "save-instructions": {
      key: "save-instructions",
      label: "Save instructions to project",
      icon: <LuSave />,
      onSelect: () => {
        append.reset();
        setSheet("save-instructions");
      },
    },
    delete: {
      key: "delete",
      label: "Delete",
      icon: <LuTrash2 />,
      tone: "danger",
      onSelect: () => {
        remove.reset();
        setSheet("delete");
      },
    },
  };

  return (
    <header className="border-b border-line px-2 py-2 sm:px-4">
      <div className="flex items-center gap-1">
        <MenuButton />
        <Title key={chat.title} chat={chat} />
        <ModelPicker
          models={models}
          modelId={chat.modelId}
          reasoning={chat.reasoning}
          toolsOn={toolsOn}
          onChange={async ({ modelId, reasoning }) => {
            saved(await updateChat(chat.id, { modelId, reasoning }));
          }}
        />
        <Menu label="Chat actions" items={menuActions(chat).map((action) => items[action])} />
      </div>
      {project ? (
        <p className="px-2 text-xs text-muted lg:pl-2">
          In{" "}
          <Link to={link.project(project.id)} className="text-link">
            {projectLabel(project)}
          </Link>
        </p>
      ) : null}
      {change.isError ? (
        <p role="alert" className="field-error px-2">
          {describeError(change.error).message}
        </p>
      ) : null}

      <Modal
        open={sheet === "instructions"}
        title="Chat Instructions"
        placement="side"
        onClose={close}
      >
        <InstructionsForm chat={chat} project={project} onDone={close} />
      </Modal>
      <Modal open={sheet === "move"} title="Move to Project" onClose={close}>
        <MoveForm chat={chat} onDone={close} />
      </Modal>
      <ConfirmDialog
        open={sheet === "delete"}
        title="Delete Chat?"
        confirm="Delete chat"
        busy={remove.isPending}
        error={remove.error ? describeError(remove.error).message : undefined}
        onConfirm={() => remove.mutate()}
        onCancel={close}
      >
        <p>{`"${chatTitle(chat)}" is deleted with its messages and attachments.`}</p>
      </ConfirmDialog>
      <ConfirmDialog
        open={sheet === "save-as-project"}
        title="Save as Project?"
        confirm="Save as project"
        tone="primary"
        busy={promote.isPending}
        error={promote.error ? describeError(promote.error).message : undefined}
        onConfirm={() => promote.mutate()}
        onCancel={close}
      >
        <p>
          A new project takes this chat's title, instructions, model, reasoning, and tools. The chat
          and its attachments move into it.
        </p>
      </ConfirmDialog>
      <ConfirmDialog
        open={sheet === "save-instructions"}
        title="Save Instructions to Project?"
        confirm="Save to project"
        tone="primary"
        busy={append.isPending}
        error={append.error ? describeError(append.error).message : undefined}
        onConfirm={() => append.mutate()}
        onCancel={close}
      >
        <p>
          {`This chat's instructions are added to the end of ${project?.name ?? "the project"}'s instructions and cleared here, so they are not sent twice.`}
        </p>
      </ConfirmDialog>
    </header>
  );
}
