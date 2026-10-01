import {
  useCallback,
  useEffect,
  useMemo,
  useRef,
  useState,
  type MouseEvent as ReactMouseEvent,
} from "react";
import { listen } from "@tauri-apps/api/event";
import { open } from "@tauri-apps/plugin-dialog";
import { revealItemInDir } from "@tauri-apps/plugin-opener";
import { api } from "./api";
import type {
  EnvironmentStatus,
  GitStatus,
  LogEntry,
  LogEvent,
  NotifyEvent,
  Snapshot,
} from "./types";
import { classifyLine } from "./components/LogView";
import { effectiveConfig } from "./lib/providers";
import {
  handleNotification,
  primeNotificationPermission,
} from "./lib/notifications";
import { Icon } from "./components/Icons";
import { Sidebar } from "./components/Sidebar";
import { ProjectHeader } from "./components/ProjectHeader";
import { TaskTree } from "./components/TaskTree";
import { DetailPanel } from "./components/DetailPanel";
import { PlannerModal } from "./components/PlannerModal";
import { NewTaskModal } from "./components/NewTaskModal";
import { EditTaskModal } from "./components/EditTaskModal";
import { DeletedTasksModal } from "./components/DeletedTasksModal";
import { PastTasksModal } from "./components/PastTasksModal";
import { SettingsModal } from "./components/SettingsModal";
import { ProjectSettingsModal } from "./components/ProjectSettingsModal";
import { ShipModal } from "./components/ShipModal";
import { MergeModal } from "./components/MergeModal";
import { GitModal } from "./components/GitModal";
import { PermissionPrompt } from "./components/PermissionPrompt";

const MAX_LOG_LINES = 4000;

/** localStorage-backed state, so panel sizes and visibility survive a reload. */
function useStoredState<T>(key: string, initial: T) {
  const [value, setValue] = useState<T>(() => {
    try {
      const raw = window.localStorage.getItem(key);
      return raw === null ? initial : (JSON.parse(raw) as T);
    } catch {
      return initial;
    }
  });
  useEffect(() => {
    try {
      window.localStorage.setItem(key, JSON.stringify(value));
    } catch {
      /* storage may be unavailable; layout just won't persist */
    }
  }, [key, value]);
  return [value, setValue] as const;
}

const PANEL_LIMITS = {
  left: { min: 190, max: 440 },
  right: { min: 280, max: 900 },
};

/** A draggable divider between a panel and the main area. */
function ResizeHandle({
  side,
  width,
  setWidth,
  onCollapse,
}: {
  side: "left" | "right";
  width: number;
  setWidth: (w: number) => void;
  onCollapse: () => void;
}) {
  function onMouseDown(e: ReactMouseEvent) {
    e.preventDefault();
    const { min, max } = PANEL_LIMITS[side];
    const startX = e.clientX;
    const startWidth = width;
    const dir = side === "left" ? 1 : -1;
    document.body.style.userSelect = "none";
    document.body.style.cursor = "col-resize";
    const move = (ev: MouseEvent) => {
      const next = startWidth + (ev.clientX - startX) * dir;
      if (next < min * 0.55) {
        onCollapse();
        return;
      }
      setWidth(Math.min(max, Math.max(min, next)));
    };
    const up = () => {
      window.removeEventListener("mousemove", move);
      window.removeEventListener("mouseup", up);
      document.body.style.userSelect = "";
      document.body.style.cursor = "";
    };
    window.addEventListener("mousemove", move);
    window.addEventListener("mouseup", up);
  }

  return (
    <div
      onMouseDown={onMouseDown}
      onDoubleClick={onCollapse}
      className="group relative z-20 w-1 shrink-0 cursor-col-resize"
      title="Drag to resize · double-click to hide"
    >
      <div className="pointer-events-none absolute inset-y-0 left-1/2 w-px -translate-x-1/2 bg-line transition-all group-hover:w-[3px] group-hover:bg-accent" />
    </div>
  );
}

/** Edge tab shown when a panel is hidden, to bring it back. */
function EdgeTab({
  side,
  onClick,
}: {
  side: "left" | "right";
  onClick: () => void;
}) {
  const show = side === "left" ? "Show projects" : "Show details";
  return (
    <button
      onClick={onClick}
      className={`no-drag absolute top-1/2 z-30 -translate-y-1/2 border border-line bg-panel py-2.5 text-ink-subtle shadow-lg backdrop-blur transition hover:text-ink ${
        side === "left" ? "left-0 rounded-r-lg" : "right-0 rounded-l-lg"
      }`}
      title={show}
      aria-label={show}
    >
      <Icon
        name="chevron"
        className={`h-3.5 w-3.5 ${side === "left" ? "rotate-180" : ""}`}
      />
    </button>
  );
}

export default function App() {
  const [snapshot, setSnapshot] = useState<Snapshot | null>(null);
  const [selectedProject, setSelectedProject] = useState<string | null>(null);
  const [selectedTaskId, setSelectedTaskId] = useState<string | null>(null);
  const [editingTaskId, setEditingTaskId] = useState<string | null>(null);
  const [logs, setLogs] = useState<Record<string, LogEntry[]>>({});
  const [status, setStatus] = useState<GitStatus | null>(null);
  const [showPlanner, setShowPlanner] = useState(false);
  const [showNewTask, setShowNewTask] = useState(false);
  const [showSettings, setShowSettings] = useState(false);
  const [showProjectSettings, setShowProjectSettings] = useState(false);
  const [showShip, setShowShip] = useState(false);
  const [showMerge, setShowMerge] = useState(false);
  const [showBranchDiff, setShowBranchDiff] = useState(false);
  const [showDeleted, setShowDeleted] = useState(false);
  const [showPast, setShowPast] = useState(false);
  const [remote, setRemote] = useState<string | null>(null);
  const [toast, setToast] = useState<string | null>(null);
  const [dismissedAskId, setDismissedAskId] = useState<string | null>(null);
  const toastTimer = useRef<number | null>(null);

  // Latest settings for the notification listener, which is registered once.
  const settingsRef = useRef<Snapshot["settings"] | undefined>(undefined);
  useEffect(() => {
    settingsRef.current = snapshot?.settings;
  }, [snapshot]);

  const [leftOpen, setLeftOpen] = useStoredState("solayge.layout.leftOpen", true);
  const [rightOpen, setRightOpen] = useStoredState("solayge.layout.rightOpen", true);
  const [leftWidth, setLeftWidth] = useStoredState("solayge.layout.leftWidth", 264);
  const [rightWidth, setRightWidth] = useStoredState("solayge.layout.rightWidth", 460);

  const [tools, setTools] = useState<EnvironmentStatus | null>(null);
  const refreshTools = useCallback(() => {
    api
      .environmentCheck()
      .then(setTools)
      .catch(() => {});
  }, []);
  useEffect(() => {
    refreshTools();
  }, [refreshTools]);

  const notify = useCallback((msg: string) => {
    setToast(msg);
    if (toastTimer.current) window.clearTimeout(toastTimer.current);
    toastTimer.current = window.setTimeout(() => setToast(null), 5000);
  }, []);

  const apply = useCallback(
    (s: Snapshot) => {
      setSnapshot(s);
      setSelectedProject((cur) => {
        if (cur && s.projects.some((p) => p.path === cur)) return cur;
        return s.projects[0]?.path ?? null;
      });
    },
    [],
  );

  // Initial load.
  useEffect(() => {
    api.snapshot().then(apply).catch((e) => notify(String(e)));
  }, [apply, notify]);

  // Ask for desktop-notification permission once (used for task + permission events).
  useEffect(() => {
    void primeNotificationPermission();
  }, []);

  // Live state + log streams, with a polling fallback.
  useEffect(() => {
    const unlisteners: (() => void)[] = [];
    listen<Snapshot>("state://changed", (e) => apply(e.payload)).then((u) =>
      unlisteners.push(u),
    );
    listen<LogEvent>("task://log", (e) => {
      const { task_id, line, kind } = e.payload;
      const entry: LogEntry = { text: line, kind: classifyLine(line, kind) };
      setLogs((prev) => {
        const arr = prev[task_id] ? [...prev[task_id], entry] : [entry];
        if (arr.length > MAX_LOG_LINES) arr.splice(0, arr.length - MAX_LOG_LINES);
        return { ...prev, [task_id]: arr };
      });
    }).then((u) => unlisteners.push(u));

    listen<NotifyEvent>("app://notify", (e) => {
      notify(e.payload.body);
      void handleNotification(e.payload, settingsRef.current?.notifications);
    }).then((u) => unlisteners.push(u));

    const poll = window.setInterval(() => {
      api.snapshot().then(apply).catch(() => {});
    }, 2500);

    return () => {
      unlisteners.forEach((u) => u());
      window.clearInterval(poll);
    };
  }, [apply]);

  // Git status for the selected project.
  const refreshStatus = useCallback(() => {
    if (!selectedProject) {
      setStatus(null);
      return;
    }
    api
      .projectStatus(selectedProject)
      .then(setStatus)
      .catch(() => setStatus(null));
  }, [selectedProject]);

  useEffect(() => {
    // Don't show the previous project's branch until the new status arrives.
    setStatus(null);
    refreshStatus();
    const t = window.setInterval(refreshStatus, 5000);
    return () => window.clearInterval(t);
  }, [refreshStatus]);

  // Load full log when a task is selected.
  useEffect(() => {
    if (!selectedTaskId) return;
    api
      .taskLog(selectedTaskId)
      .then((text) => {
        const lines = text.length ? text.replace(/\n$/, "").split("\n") : [];
        setLogs((prev) => ({
          ...prev,
          [selectedTaskId]: lines.map((line) => ({
            text: line,
            kind: classifyLine(line),
          })),
        }));
      })
      .catch(() => {});
  }, [selectedTaskId]);

  // Remote URL for the selected project (shown as a GitHub link).
  useEffect(() => {
    if (!selectedProject) {
      setRemote(null);
      return;
    }
    setRemote(null);
    let alive = true;
    api
      .projectRemote(selectedProject)
      .then((r) => alive && setRemote(r))
      .catch(() => alive && setRemote(null));
    return () => {
      alive = false;
    };
  }, [selectedProject]);

  // The detail panel is per-task and per-project; drop the selection when the
  // project changes so it never shows the previous project's task — unless the
  // selection is still valid in the new project (e.g. a permission popup opening
  // one of its tasks).
  useEffect(() => {
    setEditingTaskId(null);
    setShowDeleted(false);
    setShowPast(false);
    setSelectedTaskId((cur) => {
      if (
        cur &&
        snapshot?.tasks.some(
          (t) => t.id === cur && t.project_path === selectedProject,
        )
      ) {
        return cur;
      }
      return null;
    });
    // Only re-run when the project changes; the latest `snapshot` is read here.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [selectedProject]);

  const project = snapshot?.projects.find((p) => p.path === selectedProject) ?? null;
  const projectTasks = snapshot
    ? snapshot.tasks.filter((t) => t.project_path === selectedProject)
    : [];
  const deletedTasks = snapshot
    ? snapshot.deleted_tasks.filter(
        (d) => d.task.project_path === selectedProject,
      )
    : [];
  // A task belongs to exactly one project: only resolve the selected id within
  // the current project so switching projects never shows another project's task.
  const selectedTask =
    projectTasks.find((t) => t.id === selectedTaskId) ?? null;
  // Only a draft is editable; resolve the id within the project so switching
  // projects can never open another project's task.
  const editingTask =
    projectTasks.find((t) => t.id === editingTaskId) ?? null;
  const running = projectTasks.filter((t) => t.status === "running").length;
  const resolved = useMemo(
    () => effectiveConfig(project ?? {}, snapshot?.settings ?? {}),
    [project, snapshot?.settings],
  );

  // The first running task waiting on a permission, surfaced as an app-level
  // popup so the decision is made in-app with the directory and purpose visible.
  const pendingPermission = useMemo(() => {
    if (!snapshot) return null;
    return (
      snapshot.tasks.find(
        (t) => t.status === "running" && t.ask?.kind === "permission",
      ) ?? null
    );
  }, [snapshot]);
  const permissionProject = pendingPermission
    ? (snapshot?.projects.find(
        (p) => p.path === pendingPermission.project_path,
      ) ?? null)
    : null;
  const showPermission =
    !!pendingPermission && pendingPermission.ask?.id !== dismissedAskId;

  async function addProject() {
    try {
      const dir = await open({ directory: true, multiple: false });
      if (typeof dir === "string") {
        const s = await api.addProject(dir);
        apply(s);
        setSelectedProject(dir);
      }
    } catch (e) {
      notify(String(e));
    }
  }

  async function removeProject(path: string) {
    try {
      const s = await api.removeProject(path);
      apply(s);
      setSelectedTaskId(null);
    } catch (e) {
      notify(String(e));
    }
  }

  async function runAction(fn: () => Promise<Snapshot>) {
    try {
      apply(await fn());
    } catch (e) {
      notify(String(e));
    }
  }

  async function openEditor() {
    if (!project) return;
    try {
      await api.openInEditor(project.path);
    } catch (e) {
      notify(String(e));
    }
  }

  async function openRemote() {
    if (!remote) return;
    try {
      await api.openExternal(remote);
    } catch (e) {
      notify(String(e));
    }
  }

  if (!snapshot) {
    return (
      <div className="flex h-full items-center justify-center text-sm text-ink-subtle">
        Loading…
      </div>
    );
  }

  return (
    <div className="flex h-full">
      {leftOpen && (
        <>
          <Sidebar
            snapshot={snapshot}
            selected={selectedProject}
            width={leftWidth}
            onSelect={setSelectedProject}
            onAdd={addProject}
            onRemove={removeProject}
            onConcurrency={(n) => void runAction(() => api.setConcurrency(n))}
            onSettings={() => setShowSettings(true)}
            onReorder={(paths) => void runAction(() => api.reorderProjects(paths))}
            onCollapse={() => setLeftOpen(false)}
          />
          <ResizeHandle
            side="left"
            width={leftWidth}
            setWidth={setLeftWidth}
            onCollapse={() => setLeftOpen(false)}
          />
        </>
      )}

      <main className="relative flex min-w-0 flex-1 flex-col">
        {!leftOpen && (
          <EdgeTab side="left" onClick={() => setLeftOpen(true)} />
        )}
        {!rightOpen && project && (
          <EdgeTab side="right" onClick={() => setRightOpen(true)} />
        )}
        {project ? (
          <>
            <ProjectHeader
              project={project}
              status={status}
              tasks={projectTasks}
              running={running}
              resolved={resolved}
              remote={remote}
              tools={tools}
              onCheckTools={refreshTools}
              onNewTask={() => setShowNewTask(true)}
              onPlan={() => setShowPlanner(true)}
              onRefresh={() => {
                refreshStatus();
                api.snapshot().then(apply).catch(() => {});
              }}
              onReveal={() => void revealItemInDir(project.path)}
              onSetProfile={(p) =>
                void runAction(() => api.setProjectDefaultProfile(project.path, p))
              }
              onOpenRemote={() => void openRemote()}
              onOpenEditor={() => void openEditor()}
              onProjectSettings={() => setShowProjectSettings(true)}
              onShip={() => setShowShip(true)}
              onBranchDiff={() => setShowBranchDiff(true)}
              onExecute={() =>
                void runAction(() => api.executeProject(project.path))
              }
            />

            <div className="flex min-h-0 flex-1">
              <div className="scroll min-h-0 flex-1 px-6 py-5">
                <TaskTree
                  tasks={projectTasks}
                  selectedId={selectedTaskId}
                  onSelect={setSelectedTaskId}
                  onEdit={setEditingTaskId}
                  onStartNow={(id) =>
                    void runAction(() => api.startNow(id))
                  }
                  onCancel={(id) => void runAction(() => api.cancel(id))}
                  onRetry={(id) => void runAction(() => api.retry(id))}
                  onDelete={(id) => {
                    if (id === selectedTaskId) setSelectedTaskId(null);
                    void runAction(() => api.deleteTask(id));
                  }}
                  onShowDiff={(id) => setSelectedTaskId(id)}
                  onCombine={() => setShowMerge(true)}
                  onClearFinished={() =>
                    void runAction(() => api.clearFinished(project.path))
                  }
                  deletedCount={deletedTasks.length}
                  onShowDeleted={() => setShowDeleted(true)}
                  onShowPast={() => setShowPast(true)}
                />
              </div>

              {rightOpen && (
                <>
                  <ResizeHandle
                    side="right"
                    width={rightWidth}
                    setWidth={setRightWidth}
                    onCollapse={() => setRightOpen(false)}
                  />
                  <DetailPanel
                    task={selectedTask}
                    project={project}
                    logs={selectedTaskId ? logs[selectedTaskId] ?? [] : []}
                    width={rightWidth}
                    onEdit={setEditingTaskId}
                    onRemoveWorktree={(id) =>
                      void runAction(() => api.removeWorktree(id))
                    }
                    onRetryReview={(id) =>
                      void runAction(() => api.retryReview(id))
                    }
                    onCollapse={() => setRightOpen(false)}
                  />
                </>
              )}
            </div>
          </>
        ) : (
          <WelcomeScreen onAdd={addProject} />
        )}
      </main>

      {showPlanner && project && (
        <PlannerModal
          project={project}
          onClose={() => setShowPlanner(false)}
          onCreated={apply}
        />
      )}
      {showNewTask && project && snapshot && (
        <NewTaskModal
          project={project}
          tasks={projectTasks}
          onClose={() => setShowNewTask(false)}
          onCreated={apply}
        />
      )}
      {editingTask && editingTask.status === "draft" && project && (
        <EditTaskModal
          task={editingTask}
          project={project}
          tasks={projectTasks}
          onClose={() => setEditingTaskId(null)}
          onSaved={apply}
        />
      )}
      {showDeleted && project && (
        <DeletedTasksModal
          deleted={deletedTasks}
          onClose={() => setShowDeleted(false)}
          onRestore={(id) => void runAction(() => api.restoreTask(id))}
        />
      )}
      {showPast && project && (
        <PastTasksModal
          deleted={deletedTasks}
          projectName={project.name}
          onClose={() => setShowPast(false)}
        />
      )}
      {showSettings && snapshot && (
        <SettingsModal
          snapshot={snapshot}
          tools={tools}
          onClose={() => setShowSettings(false)}
          onSaved={(s) => {
            apply(s);
            refreshTools();
          }}
        />
      )}
      {showProjectSettings && project && snapshot && (
        <ProjectSettingsModal
          project={project}
          settings={snapshot.settings}
          tools={tools}
          onClose={() => setShowProjectSettings(false)}
          onSaved={apply}
        />
      )}
      {showShip && project && (
        <ShipModal
          project={project}
          onClose={() => setShowShip(false)}
          onCreated={apply}
        />
      )}
      {showMerge && project && (
        <MergeModal
          project={project}
          tasks={projectTasks}
          onClose={() => setShowMerge(false)}
          onCreated={apply}
        />
      )}
      {showBranchDiff && project && (
        <GitModal
          project={project}
          onClose={() => setShowBranchDiff(false)}
        />
      )}

      {showPermission && pendingPermission && (
        <PermissionPrompt
          key={pendingPermission.ask?.id}
          task={pendingPermission}
          projectName={permissionProject?.name ?? null}
          onAnswered={apply}
          onDismiss={() => setDismissedAskId(pendingPermission.ask?.id ?? null)}
          onOpenTask={() => {
            setSelectedProject(pendingPermission.project_path);
            setSelectedTaskId(pendingPermission.id);
            setRightOpen(true);
            setDismissedAskId(pendingPermission.ask?.id ?? null);
          }}
        />
      )}

      {toast && (
        <div className="fixed bottom-5 left-1/2 z-50 flex max-w-xl -translate-x-1/2 items-center gap-2 rounded-lg border border-line bg-panel px-4 py-2 text-[12px] text-ink shadow-xl backdrop-blur">
          <Icon name="zap" className="h-3.5 w-3.5 shrink-0 text-accent" />
          <span className="truncate">{toast}</span>
        </div>
      )}
    </div>
  );
}

function WelcomeScreen({ onAdd }: { onAdd: () => void }) {
  return (
    <div
      className="drag-region flex h-full flex-col items-center justify-center gap-5 px-10"
      data-tauri-drag-region="deep"
    >
      <img src="/app-icon.png" alt="" className="h-16 w-16 rounded-2xl shadow-2xl" />
      <div className="text-center">
        <h1 className="text-xl font-semibold text-ink">Solayge</h1>
        <p className="mx-auto mt-2 max-w-md text-sm leading-relaxed text-ink-muted">
          Pick a git project, plan a chain of sequential tasks or a parallel tree,
          and run each one in its own worktree. Watch progress, stream logs, and
          review diffs — all in one window.
        </p>
      </div>
      <button className="btn btn-primary no-drag !px-4 !py-2" onClick={onAdd}>
        <Icon name="folder" className="h-4 w-4" />
        Add a project folder
      </button>
    </div>
  );
}
