import { useCallback, useEffect, useState, type ReactNode } from "react";
import {
  Activity,
  ArrowDownToLine,
  ArrowLeft,
  ArrowRight,
  Box,
  Boxes,
  Cable,
  Check,
  ChevronRight,
  Circle,
  Container as ContainerIcon,
  Copy,
  Database,
  FolderOpen,
  LayoutDashboard,
  Menu,
  Network as NetworkIcon,
  Play,
  Plus,
  RefreshCw,
  Search,
  Settings,
  ShieldCheck,
  Star,
  TerminalSquare,
  Trash2,
  TriangleAlert,
  X,
} from "lucide-react";
import { api, setToken } from "./api";
import type {
  Container,
  EngineEvent,
  Inventory,
  Job,
  Json,
  Port,
  Project,
  Session,
} from "./types";
import Editor from "./Editor";
import Logs from "./Logs";
import Terminal from "./Terminal";

type Page =
  | "dashboard"
  | "containers"
  | "ports"
  | "images"
  | "networks"
  | "volumes"
  | "activity"
  | "settings";
type Selection =
  | { kind: "project"; project: Project }
  | { kind: "container"; container: Container };
type Modal = { title: string; content: ReactNode };
const labels: Record<string, string> = {
  running: "En cours",
  stopped: "Arrêté",
  exited: "Arrêté",
  discovered: "Découvert",
  partial: "Partiel",
  invalid: "Invalide",
  runtime_only: "Runtime uniquement",
  healthy: "Sain",
  unhealthy: "Dégradé",
  queued: "En attente",
  succeeded: "Terminé",
  failed: "Échec",
  docker_occupied: "Occupé par Docker",
  stopped_reservation: "Réservé · arrêté",
  desired: "Défini dans YAML",
  external_detected: "Service externe",
  probably_free: "Probablement libre",
  unknown: "Inconnu",
  update_available: "Mise à jour disponible",
  up_to_date: "À jour",
  local: "Image locale",
};
const title: Record<Page, string> = {
  dashboard: "Vos projets, en un regard.",
  containers: "Conteneurs autonomes",
  ports: "Cartographie des ports",
  images: "Images",
  networks: "Réseaux",
  volumes: "Volumes",
  activity: "Historique des opérations",
  settings: "Configuration",
};
const nav: { id: Page; label: string; icon: typeof Box }[] = [
  { id: "dashboard", label: "Vue d’ensemble", icon: LayoutDashboard },
  { id: "containers", label: "Conteneurs", icon: ContainerIcon },
  { id: "ports", label: "Ports", icon: Cable },
  { id: "images", label: "Images", icon: Boxes },
  { id: "networks", label: "Réseaux", icon: NetworkIcon },
  { id: "volumes", label: "Volumes", icon: Database },
  { id: "activity", label: "Activité", icon: Activity },
];
const stateTone = (value: string) =>
  ["running", "healthy", "succeeded", "up_to_date"].includes(value)
    ? "green"
    : ["invalid", "unhealthy", "failed"].includes(value)
      ? "red"
      : ["runtime_only", "queued", "update_available"].includes(value)
        ? "amber"
        : "neutral";
function Badge({ value }: { value: string }) {
  return (
    <span className={`badge ${stateTone(value)}`}>
      <i />
      {labels[value] || value}
    </span>
  );
}
function Empty({ children }: { children: ReactNode }) {
  return (
    <div className="empty">
      <FolderOpen size={32} />
      <p>{children}</p>
    </div>
  );
}
function bytes(value: number) {
  return (
    new Intl.NumberFormat("fr-FR", { maximumFractionDigits: 1 }).format(
      value / 1024 / 1024,
    ) + " Mo"
  );
}
function time(value: string) {
  return new Date(value).toLocaleString("fr-FR", {
    day: "2-digit",
    month: "short",
    hour: "2-digit",
    minute: "2-digit",
    second: "2-digit",
  });
}

export default function App() {
  const [session, setSession] = useState<Session | null>(null);
  const [inventory, setInventory] = useState<Inventory | null>(null);
  const [jobs, setJobs] = useState<Job[]>([]);
  const [page, setPage] = useState<Page>("dashboard");
  const [selection, setSelection] = useState<Selection | null>(null);
  const [tab, setTab] = useState("overview");
  const [search, setSearch] = useState("");
  const [filter, setFilter] = useState("all");
  const [favoriteOnly, setFavoriteOnly] = useState(false);
  const [preferencesReady, setPreferencesReady] = useState(false);
  const [mobile, setMobile] = useState(false);
  const [toast, setToast] = useState<{ text: string; error: boolean } | null>(
    null,
  );
  const [modal, setModal] = useState<Modal | null>(null);
  const [busy, setBusy] = useState(false);
  const [suggested, setSuggested] = useState<
    { from: number; to: number } | undefined
  >();
  const report = useCallback(
    (text: string, error = false) => setToast({ text, error }),
    [],
  );
  const load = useCallback(async () => {
    try {
      const [inv, history] = await Promise.all([
        api<Inventory>("/inventory"),
        api<Job[]>("/jobs"),
      ]);
      setInventory(inv);
      setJobs(history);
    } catch (e) {
      report((e as Error).message, true);
    }
  }, [report]);
  useEffect(() => {
    api<Session>("/session")
      .then((data) => {
        setToken(data.csrfToken);
        setSession(data);
        void load();
        void api<{
          filters?: {
            page?: Page;
            search?: string;
            state?: string;
            favoriteOnly?: boolean;
          };
        }>("/settings")
          .then((settings) => {
            const filters = settings.filters;
            if (
              filters?.page &&
              (nav.some((item) => item.id === filters.page) ||
                filters.page === "settings")
            )
              setPage(filters.page);
            if (typeof filters?.search === "string") setSearch(filters.search);
            if (typeof filters?.state === "string") setFilter(filters.state);
            if (typeof filters?.favoriteOnly === "boolean")
              setFavoriteOnly(filters.favoriteOnly);
          })
          .catch(() => {})
          .finally(() => setPreferencesReady(true));
      })
      .catch((e) => report(e.message, true));
    const source = new EventSource("/api/v1/events");
    let generation = "";
    source.addEventListener("inventory", (event) => {
      if (event.data !== generation) {
        generation = event.data;
        void load();
      }
    });
    source.addEventListener("job", (event) => {
      const job = JSON.parse(event.data) as Job;
      setJobs((jobs) => [job, ...jobs.filter((j) => j.id !== job.id)]);
      if (["succeeded", "failed"].includes(job.state)) {
        report(
          job.error || `Opération ${job.action} terminée`,
          job.state === "failed",
        );
        void load();
      }
    });
    return () => source.close();
  }, [load, report]);
  useEffect(() => {
    if (!preferencesReady) return;
    const timer = setTimeout(() => {
      void api("/settings", "PUT", {
        filters: { page, search, state: filter, favoriteOnly },
      }).catch((e) => report(e.message, true));
    }, 400);
    return () => clearTimeout(timer);
  }, [preferencesReady, page, search, filter, favoriteOnly, report]);
  useEffect(() => {
    if (!toast) return;
    const timeout = setTimeout(
      () => setToast(null),
      toast.error ? 16000 : 6000,
    );
    return () => clearTimeout(timeout);
  }, [toast]);
  const go = (next: Page) => {
    setPage(next);
    setSelection(null);
    setSearch("");
    setFilter("all");
    setMobile(false);
  };
  const open = (next: Selection) => {
    setSelection(next);
    setTab("overview");
    setSuggested(undefined);
  };
  const refresh = async () => {
    setBusy(true);
    try {
      await api("/refresh", "POST");
      await load();
    } catch (e) {
      report((e as Error).message, true);
    } finally {
      setBusy(false);
    }
  };
  const submit = async (
    target: string,
    action: string,
    options: object = {},
  ) => {
    try {
      const job = await api<Job>("/jobs", "POST", { target, action, options });
      setJobs((j) => [job, ...j]);
      report(`Opération ${action} mise en file d’attente`);
    } catch (e) {
      report((e as Error).message, true);
    }
  };
  const confirmAction = (
    target: string,
    action: string,
    name: string,
    options: object = {},
  ) => {
    setModal({
      title: `Confirmer : ${action}`,
      content: (
        <>
          <p>
            Appliquer <strong>{action}</strong> à <strong>{name}</strong> ?
          </p>
          <p className="muted">
            {action === "down"
              ? "Les conteneurs et réseaux du projet seront supprimés. Les volumes sont conservés."
              : action === "remove"
                ? "Cette suppression est définitive. Docker refusera les ressources encore utilisées."
                : "Cette action peut interrompre le service."}
          </p>
          <div className="modal-actions">
            <button className="ghost" onClick={() => setModal(null)}>
              Annuler
            </button>
            <button
              className="danger"
              onClick={() => {
                setModal(null);
                void submit(target, action, { ...options, confirmed: true });
              }}
            >
              Confirmer
            </button>
          </div>
        </>
      ),
    });
  };
  const run = (target: string, action: string, name: string) => {
    if (
      [
        "down",
        "stop",
        "restart",
        "remove",
        "kill",
        "pause",
        "unpause",
      ].includes(action)
    )
      confirmAction(target, action, name);
    else void submit(target, action);
  };
  const inspect = async (kind: string, id: string) => {
    try {
      const value = await api<Json>(
        `/inspect/${kind}/${encodeURIComponent(id)}`,
      );
      setModal({
        title: `Inspection · ${kind}`,
        content: (
          <pre className="json-view">{JSON.stringify(value, null, 2)}</pre>
        ),
      });
    } catch (e) {
      report((e as Error).message, true);
    }
  };
  const resourceForm = (kind: "network" | "volume") =>
    setModal({
      title: `Créer un ${kind === "network" ? "réseau" : "volume"}`,
      content: (
        <form
          onSubmit={(e) => {
            e.preventDefault();
            const data = new FormData(e.currentTarget);
            setModal(null);
            void submit(`${kind}:new`, `create_${kind}`, {
              name: data.get("name"),
              driver: data.get("driver"),
              subnet: data.get("subnet"),
              gateway: data.get("gateway"),
              internal: data.get("internal") === "on",
              ipv6: data.get("ipv6") === "on",
              attachable: data.get("attachable") === "on",
            });
          }}
        >
          <label>
            Nom
            <input name="name" required pattern="[a-zA-Z0-9][a-zA-Z0-9_.-]*" />
          </label>
          <label>
            Driver
            <input
              name="driver"
              defaultValue={kind === "network" ? "bridge" : "local"}
              required
            />
          </label>
          {kind === "network" && (
            <>
              <label>
                Sous-réseau (facultatif)
                <input name="subnet" placeholder="172.28.0.0/16" />
              </label>
              <label>
                Passerelle (facultatif)
                <input name="gateway" placeholder="172.28.0.1" />
              </label>
              <label className="checkbox-row">
                <input type="checkbox" name="internal" />
                Réseau interne
              </label>
              <label className="checkbox-row">
                <input type="checkbox" name="ipv6" />
                IPv6
              </label>
              <label className="checkbox-row">
                <input type="checkbox" name="attachable" />
                Attachable
              </label>
            </>
          )}
          <button className="primary">Créer</button>
        </form>
      ),
    });
  const networkLink = (id: string, action: "connect" | "disconnect") =>
    setModal({
      title:
        action === "connect"
          ? "Connecter un conteneur"
          : "Déconnecter un conteneur",
      content: (
        <form
          onSubmit={(e) => {
            e.preventDefault();
            const container = new FormData(e.currentTarget).get("container");
            setModal(null);
            void submit(`network:${id}`, action, {
              container,
              confirmed: true,
            });
          }}
        >
          <label>
            Conteneur
            <select name="container">
              {allContainers.map((c) => (
                <option key={c.id} value={c.id}>
                  {c.name}
                </option>
              ))}
            </select>
          </label>
          <button className="primary">Confirmer</button>
        </form>
      ),
    });
  const prune = async () => {
    try {
      const resources =
        await api<{ kind: string; id: string }[]>("/prune/preview");
      setModal({
        title: "Nettoyage ciblé",
        content: (
          <form
            onSubmit={async (e) => {
              e.preventDefault();
              const data = new FormData(e.currentTarget);
              const chosen = resources.filter((_, i) =>
                data.get(`resource-${i}`),
              );
              if (!chosen.length) return;
              try {
                const job = await api<Job>("/prune", "POST", {
                  confirmed: true,
                  resources: chosen,
                });
                setModal(null);
                setJobs((jobs) => [job, ...jobs]);
                report(
                  `Nettoyage de ${chosen.length} ressource(s) mis en file d’attente`,
                );
                await load();
              } catch (e) {
                report((e as Error).message, true);
              }
            }}
          >
            <p>
              Seules les ressources inutilisées sélectionnées seront supprimées.
              Les volumes peuvent contenir des données importantes.
            </p>
            <div className="prune-list">
              {resources.length ? (
                resources.map((r, i) => (
                  <label className="checkbox-row" key={`${r.kind}:${r.id}`}>
                    <input type="checkbox" name={`resource-${i}`} />
                    <Badge value={r.kind} />
                    <code>{r.id}</code>
                  </label>
                ))
              ) : (
                <p className="muted">Aucune ressource éligible.</p>
              )}
            </div>
            <button className="danger" disabled={!resources.length}>
              <Trash2 size={15} />
              Supprimer la sélection
            </button>
          </form>
        ),
      });
    } catch (e) {
      report((e as Error).message, true);
    }
  };
  const allContainers = inventory
    ? [
        ...inventory.standalone,
        ...inventory.projects.flatMap((p) => p.containers),
      ]
    : [];
  const selectedProject =
    selection?.kind === "project"
      ? inventory?.projects.find((p) => p.id === selection.project.id) ||
        selection.project
      : null;
  const selectedContainer =
    selection?.kind === "container"
      ? allContainers.find((c) => c.id === selection.container.id) ||
        selection.container
      : null;
  const selectedName = selectedProject?.name || selectedContainer?.name || "";
  const selectedId = selectedProject?.id || selectedContainer?.id || "";
  const selectedContainers =
    selectedProject?.containers ||
    (selectedContainer ? [selectedContainer] : []);
  const matches = (text: string) =>
    text.toLowerCase().includes(search.toLowerCase());
  const pending = jobs.filter((j) =>
    ["queued", "running"].includes(j.state),
  ).length;

  return (
    <div className="app-shell">
      {mobile && (
        <div className="sidebar-scrim" onClick={() => setMobile(false)} />
      )}
      <aside className={`sidebar ${mobile ? "open" : ""}`}>
        <a
          className="brand"
          href="#"
          onClick={(e) => {
            e.preventDefault();
            go("dashboard");
          }}
        >
          <span className="brand-mark">
            <Boxes size={23} />
          </span>
          <span>
            comp<span className="brand-light">master</span>
            <small>LOCAL DOCKER CONTROL</small>
          </span>
        </a>
        <div className="workspace-label">ESPACE DE TRAVAIL</div>
        <nav>
          {nav.map((item) => (
            <button
              key={item.id}
              className={`nav-item ${page === item.id && !selection ? "active" : ""}`}
              onClick={() => go(item.id)}
            >
              <item.icon size={18} />
              <span>{item.label}</span>
              {item.id === "dashboard" && (
                <small>{inventory?.summary.projects || 0}</small>
              )}
              {item.id === "activity" && pending > 0 && (
                <small className="pending-count">{pending}</small>
              )}
            </button>
          ))}
        </nav>
        <div className="sidebar-bottom">
          <button className="nav-item" onClick={() => go("settings")}>
            <Settings size={18} />
            Configuration
          </button>
          <div className="engine-box">
            <span
              className={`status-dot ${inventory?.engineConnected ? "online" : ""}`}
            />
            <div>
              <strong>
                {inventory?.engineConnected
                  ? "Docker connecté"
                  : "Docker indisponible"}
              </strong>
              <small>Moteur local · {session?.version || "0.1.0"}</small>
            </div>
            <ShieldCheck size={17} />
          </div>
        </div>
      </aside>
      <main>
        <header className="topbar">
          <div className="breadcrumbs">
            <button
              className="icon-button mobile-menu"
              aria-label="Menu"
              onClick={() => setMobile(!mobile)}
            >
              <Menu size={21} />
            </button>
            <span>Workspace</span>
            <ChevronRight size={14} />
            <strong>
              {selection
                ? selectedName
                : nav.find((n) => n.id === page)?.label || "Configuration"}
            </strong>
          </div>
          <div className="topbar-right">
            <span className="local-pill">
              <Circle size={8} fill="currentColor" /> LOCAL
            </span>
            <button
              className={`icon-button ${busy ? "spinning" : ""}`}
              aria-label="Actualiser"
              disabled={busy}
              onClick={() => void refresh()}
            >
              <RefreshCw size={17} />
            </button>
            <span className="avatar">CM</span>
          </div>
        </header>
        <div className="page">
          <div className="page-heading">
            <div>
              {selection ? (
                <button
                  className="back-button"
                  onClick={() => setSelection(null)}
                >
                  <ArrowLeft size={15} />
                  Retour à la liste
                </button>
              ) : (
                <div className="eyebrow">
                  COMPMASTER /{" "}
                  {page === "dashboard" ? "OVERVIEW" : page.toUpperCase()}
                </div>
              )}
              <h1>{selection ? selectedName : title[page]}</h1>
              <p>
                {selection
                  ? selectedProject?.definition?.relativePath ||
                    (selectedProject
                      ? "Déploiement Compose sans fichiers vérifiés"
                      : selectedContainer?.image)
                  : page === "dashboard"
                    ? "Découvrez, observez et pilotez vos stacks Docker depuis un seul endroit."
                    : page === "ports"
                      ? "Croisez les ports déclarés, publiés et occupés avant de démarrer."
                      : "Gardez le contrôle sur les ressources de votre moteur local."}
              </p>
            </div>
            <div className="heading-actions">
              {!selection && ["dashboard", "containers"].includes(page) && (
                <button onClick={() => void refresh()} disabled={busy}>
                  <RefreshCw size={15} />
                  Rescanner
                </button>
              )}
              {["networks", "volumes"].includes(page) && !selection && (
                <button
                  className="primary"
                  onClick={() =>
                    resourceForm(page === "networks" ? "network" : "volume")
                  }
                >
                  <Plus size={16} />
                  Créer
                </button>
              )}
              {!selection &&
                ["images", "networks", "volumes"].includes(page) && (
                  <button onClick={() => void prune()}>
                    <Trash2 size={15} />
                    Nettoyage ciblé
                  </button>
                )}
            </div>
          </div>
          {inventory && !inventory.engineConnected && (
            <div className="engine-warning">
              <TriangleAlert size={19} />
              <div>
                <strong>Moteur Docker inaccessible</strong>
                <p>{inventory.engineError}</p>
                <small>
                  Les fichiers découverts restent consultables. Vérifiez le
                  socket et ses permissions.
                </small>
              </div>
            </div>
          )}
          {!inventory ? (
            <Empty>Connexion à CompMaster…</Empty>
          ) : selection ? (
            <>
              <div className="detail-actions">
                <Badge
                  value={
                    selectedProject?.state ||
                    selectedContainer?.state ||
                    "unknown"
                  }
                />
                <span className="grow" />
                {(selectedProject
                  ? [
                      "pull",
                      "build",
                      "up",
                      "up_build",
                      "start",
                      "stop",
                      "restart",
                      "down",
                    ]
                  : [
                      "start",
                      "stop",
                      "restart",
                      "pause",
                      "unpause",
                      "kill",
                      "rename",
                      "replace",
                      "remove",
                    ]
                ).map((action) => (
                  <button
                    key={action}
                    disabled={
                      !!selectedProject &&
                      (!selectedProject.definition?.valid || false)
                    }
                    className={
                      action === "up"
                        ? "primary"
                        : ["down", "remove", "kill"].includes(action)
                          ? "danger ghost"
                          : ""
                    }
                    onClick={() => {
                      if (action === "rename")
                        setModal({
                          title: "Renommer le conteneur",
                          content: (
                            <form
                              onSubmit={(e) => {
                                e.preventDefault();
                                const name = new FormData(e.currentTarget).get(
                                  "name",
                                );
                                setModal(null);
                                void submit(selectedId, "rename", {
                                  name,
                                  confirmed: true,
                                });
                              }}
                            >
                              <label>
                                Nouveau nom
                                <input
                                  name="name"
                                  defaultValue={selectedName}
                                  required
                                />
                              </label>
                              <button className="primary">Confirmer</button>
                            </form>
                          ),
                        });
                      else if (action === "replace" && selectedContainer)
                        setModal({
                          title: "Remplacement sécurisé",
                          content: (
                            <Replacement
                              id={selectedContainer.id}
                              report={report}
                              onSubmit={(options) => {
                                setModal(null);
                                void submit(selectedId, "replace", {
                                  ...options,
                                  confirmed: true,
                                });
                              }}
                            />
                          ),
                        });
                      else run(selectedId, action, selectedName);
                    }}
                  >
                    {action === "up" || action === "start" ? (
                      <Play size={14} />
                    ) : action === "pull" ? (
                      <ArrowDownToLine size={14} />
                    ) : action === "restart" ? (
                      <RefreshCw size={14} />
                    ) : null}
                    {
                      (
                        {
                          up: "Up -d",
                          up_build: "Build & Up -d",
                          pull: "Pull",
                          build: "Build",
                          start: "Démarrer",
                          stop: "Arrêter",
                          restart: "Restart",
                          down: "Down",
                          pause: "Pause",
                          unpause: "Reprendre",
                          kill: "Kill",
                          rename: "Renommer",
                          replace: "Modifier / remplacer",
                          remove: "Supprimer",
                        } as Record<string, string>
                      )[action]
                    }
                  </button>
                ))}
              </div>
              <div className="tabs">
                {[
                  "overview",
                  ...(selectedProject?.definition ? ["yaml"] : []),
                  "logs",
                  "terminal",
                ].map((t) => (
                  <button
                    className={tab === t ? "selected" : ""}
                    onClick={() => setTab(t)}
                    key={t}
                  >
                    {
                      (
                        {
                          overview: "Vue d’ensemble",
                          yaml: "Compose YAML",
                          logs: "Logs",
                          terminal: "Terminal",
                        } as Record<string, string>
                      )[t]
                    }
                  </button>
                ))}
              </div>
              {tab === "overview" && (
                <>
                  <div className="detail-grid">
                    <div className="panel">
                      <div className="panel-heading">
                        <h2>Services & conteneurs</h2>
                        <span className="count">
                          {selectedContainers.length}
                        </span>
                      </div>
                      {selectedContainers.length ? (
                        selectedContainers.map((c) => (
                          <ContainerRow
                            key={c.id}
                            container={c}
                            open={() =>
                              open({ kind: "container", container: c })
                            }
                            inspect={() => void inspect("container", c.id)}
                            report={report}
                          />
                        ))
                      ) : (
                        <Empty>
                          Aucun conteneur créé. Lancez le projet avec Up -d.
                        </Empty>
                      )}
                    </div>
                    <div className="panel">
                      <div className="panel-heading">
                        <h2>Informations</h2>
                      </div>
                      <div className="info-list">
                        <div>
                          <span>Origine</span>
                          <strong>
                            {selectedProject
                              ? "Docker Compose"
                              : "Conteneur autonome"}
                          </strong>
                        </div>
                        <div>
                          <span>Identifiant</span>
                          <code>{selectedId.slice(0, 24)}</code>
                        </div>
                        <div>
                          <span>Services</span>
                          <strong>
                            {selectedProject?.services.join(", ") || "—"}
                          </strong>
                        </div>
                        <div>
                          <span>Conteneurs</span>
                          <strong>{selectedContainers.length}</strong>
                        </div>
                      </div>
                      {selectedProject?.definition?.diagnostics.map((d, i) => (
                        <div className="callout warning" key={i}>
                          {d.message}
                        </div>
                      ))}
                    </div>
                  </div>
                  <div className="panel">
                    <div className="panel-heading">
                      <h2>Ports & connectivité</h2>
                    </div>
                    <PortTable
                      ports={inventory.ports.filter(
                        (p) => p.ownerId === selectedId,
                      )}
                      onOwner={() => {}}
                      suggest={(port) => {
                        if (
                          selectedProject?.definition &&
                          port.conflict?.suggestedPort
                        ) {
                          setSuggested({
                            from: port.hostPort,
                            to: port.conflict.suggestedPort,
                          });
                          setTab("yaml");
                        }
                      }}
                    />
                  </div>
                  <div className="panel">
                    <div className="panel-heading">
                      <h2>Volumes & montages</h2>
                    </div>
                    <div className="table-scroll">
                      <table>
                        <thead>
                          <tr>
                            <th>Source</th>
                            <th>Destination</th>
                            <th>Type</th>
                            <th>Accès</th>
                          </tr>
                        </thead>
                        <tbody>
                          {selectedContainers.flatMap((c) =>
                            c.mounts.map((m, i) => (
                              <tr key={`${c.id}-${i}`}>
                                <td>
                                  <code>{m.source}</code>
                                </td>
                                <td>
                                  <code>{m.destination}</code>
                                </td>
                                <td>{m.kind}</td>
                                <td>
                                  {m.readOnly
                                    ? "Lecture seule"
                                    : "Lecture / écriture"}
                                </td>
                              </tr>
                            )),
                          )}
                        </tbody>
                      </table>
                    </div>
                  </div>
                </>
              )}
              {tab === "yaml" && selectedProject?.definition && (
                <Editor
                  id={selectedProject.id}
                  suggested={suggested}
                  report={report}
                  saved={() => void load()}
                />
              )}
              {tab === "overview" && (
                <>
                  <div className="panel">
                    <div className="panel-heading">
                      <h2>Adresses réseau</h2>
                    </div>
                    <div className="info-list">
                      {selectedContainers.flatMap((c) =>
                        Object.entries(c.networkAddresses || {}).map(
                          ([network, addresses]) => (
                            <div key={`${c.id}:${network}`}>
                              <span>
                                {c.name} / {network}
                              </span>
                              <code>
                                {addresses.join(", ") || "Non attribuée"}
                              </code>
                            </div>
                          ),
                        ),
                      )}
                    </div>
                  </div>
                  <EngineEvents
                    events={(inventory.events || []).filter((event) =>
                      selectedContainers.some((c) => c.id === event.id),
                    )}
                  />
                </>
              )}
              {tab === "logs" && <Logs containers={selectedContainers} />}
              {tab === "terminal" && (
                <TerminalSelector containers={selectedContainers} />
              )}
            </>
          ) : (
            <>
              {page === "dashboard" && (
                <>
                  <div className="metrics">
                    <Metric
                      label="PROJETS COMPOSE"
                      value={inventory.summary.projects}
                      detail="Détectés automatiquement"
                      icon={<FolderOpen size={19} />}
                    />
                    <Metric
                      label="PROJETS EN COURS"
                      value={inventory.summary.running}
                      detail="Déploiements actifs"
                      icon={<Activity size={19} />}
                      tone="green"
                    />
                    <Metric
                      label="CONTENEURS AUTONOMES"
                      value={inventory.summary.standalone}
                      detail="Hors Docker Compose"
                      icon={<Box size={19} />}
                    />
                    <Metric
                      label="CONFLITS DE PORTS"
                      value={inventory.summary.portConflicts}
                      detail={
                        inventory.summary.portConflicts
                          ? "À vérifier avant démarrage"
                          : "Aucun conflit détecté"
                      }
                      icon={<Cable size={19} />}
                      tone={inventory.summary.portConflicts ? "amber" : "green"}
                    />
                  </div>
                  <div className="section-heading">
                    <div>
                      <h2>
                        Projets Compose{" "}
                        <span className="count">
                          {inventory.projects.length}
                        </span>
                      </h2>
                      <p>Une découverte récursive. Aucun import manuel.</p>
                    </div>
                    <div className="root-path">
                      <FolderOpen size={14} />
                      <code title={session?.root}>{session?.root || "…"}</code>
                    </div>
                  </div>
                </>
              )}
              {[
                "dashboard",
                "containers",
                "ports",
                "images",
                "networks",
                "volumes",
              ].includes(page) && (
                <div className="filterbar">
                  <div className="searchbox">
                    <Search size={17} />
                    <input
                      aria-label="Rechercher"
                      value={search}
                      onChange={(e) => setSearch(e.target.value)}
                      placeholder={
                        page === "ports"
                          ? "Rechercher un port, un projet, une IP…"
                          : "Rechercher par nom, image ou chemin…"
                      }
                    />
                    <kbd>⌕</kbd>
                  </div>
                  {["dashboard", "containers"].includes(page) && (
                    <select
                      aria-label="Filtrer par état"
                      value={filter}
                      onChange={(e) => setFilter(e.target.value)}
                    >
                      <option value="all">Tous les états</option>
                      {(page === "containers"
                        ? [
                            "running",
                            "stopped",
                            "paused",
                            "created",
                            "unhealthy",
                            "conflicts",
                            "updates",
                          ]
                        : [
                            "running",
                            "stopped",
                            "discovered",
                            "invalid",
                            "runtime_only",
                            "unhealthy",
                            "conflicts",
                            "updates",
                          ]
                      ).map((s) => (
                        <option key={s} value={s}>
                          {labels[s] ||
                            (
                              {
                                conflicts: "Conflits de ports",
                                updates: "Image obsolète",
                              } as Record<string, string>
                            )[s]}
                        </option>
                      ))}
                    </select>
                  )}
                  {page === "ports" && (
                    <select
                      aria-label="Filtrer les ports"
                      value={filter}
                      onChange={(e) => setFilter(e.target.value)}
                    >
                      <option value="all">Tous les ports</option>
                      <option value="conflicts">Avec conflit</option>
                      <option value="docker_occupied">
                        Occupés par Docker
                      </option>
                      <option value="stopped_reservation">
                        Conteneurs arrêtés
                      </option>
                      <option value="desired">Définis dans YAML</option>
                    </select>
                  )}
                  {page === "dashboard" && (
                    <button
                      className={
                        favoriteOnly ? "favorite-button on" : "favorite-button"
                      }
                      onClick={() => setFavoriteOnly(!favoriteOnly)}
                    >
                      <Star
                        size={16}
                        fill={favoriteOnly ? "currentColor" : "none"}
                      />
                      Favoris
                    </button>
                  )}
                </div>
              )}
              {page === "dashboard" && (
                <div className="project-grid">
                  {inventory.projects
                    .filter(
                      (p) =>
                        matches(
                          `${p.name} ${p.definition?.relativePath || ""}`,
                        ) &&
                        (!favoriteOnly || p.favorite) &&
                        projectFilter(p, filter, inventory),
                    )
                    .map((project) => (
                      <article className="project-card" key={project.id}>
                        <div className="card-top">
                          <div
                            className={`project-icon ${project.state === "running" ? "active" : ""}`}
                          >
                            <Boxes size={24} />
                          </div>
                          <Badge value={project.state} />
                          <button
                            className={`icon-button favorite ${project.favorite ? "on" : ""}`}
                            aria-label={`${project.favorite ? "Retirer des" : "Ajouter aux"} favoris ${project.name}`}
                            onClick={() => {
                              void api("/settings", "PUT", {
                                [`favorite:${project.id}`]: !project.favorite,
                              })
                                .then(load)
                                .catch((e) => report(e.message, true));
                            }}
                          >
                            <Star
                              size={17}
                              fill={project.favorite ? "currentColor" : "none"}
                            />
                          </button>
                        </div>
                        <button
                          className="card-link"
                          onClick={() => open({ kind: "project", project })}
                        >
                          <h3>{project.name}</h3>
                          <p title={project.definition?.relativePath}>
                            {project.definition?.relativePath ||
                              "Fichiers source non vérifiés"}
                          </p>
                        </button>
                        <div className="card-services">
                          <span>
                            <ContainerIcon size={14} />
                            {project.services.length} service
                            {project.services.length > 1 ? "s" : ""}
                          </span>
                          <span>
                            <Circle
                              size={7}
                              fill="currentColor"
                              className={
                                project.containers.some(
                                  (c) => c.state === "running",
                                )
                                  ? "green-text"
                                  : ""
                              }
                            />
                            {
                              project.containers.filter(
                                (c) => c.state === "running",
                              ).length
                            }{" "}
                            actif(s)
                          </span>
                        </div>
                        <div className="card-footer">
                          <div>
                            {inventory.ports
                              .filter((p) => p.ownerId === project.id)
                              .slice(0, 3)
                              .map((port) => (
                                <span
                                  className={`port-chip ${port.conflict ? "warn" : ""}`}
                                  key={port.id}
                                >
                                  :{port.hostPort}
                                </span>
                              ))}
                            {!inventory.ports.some(
                              (p) => p.ownerId === project.id,
                            ) && (
                              <small className="muted">Aucun port publié</small>
                            )}
                          </div>
                          <button
                            className="icon-button card-open"
                            aria-label={`Ouvrir ${project.name}`}
                            onClick={() => open({ kind: "project", project })}
                          >
                            <ArrowRight size={17} />
                          </button>
                        </div>
                      </article>
                    ))}
                  {inventory.projects.length === 0 && (
                    <div className="panel full-width">
                      <Empty>
                        Aucun projet Compose trouvé dans {session?.root}.
                        Choisissez la racine dans le fichier .env et rescanner.
                      </Empty>
                    </div>
                  )}
                </div>
              )}
              {page === "containers" && (
                <div className="panel">
                  {inventory.standalone
                    .filter(
                      (c) =>
                        matches(`${c.name} ${c.image}`) &&
                        (filter === "all" ||
                          c.state === filter ||
                          (filter === "stopped" && c.state === "exited") ||
                          (filter === "unhealthy" &&
                            c.health === "unhealthy") ||
                          (filter === "conflicts" &&
                            inventory.ports.some(
                              (p) => p.ownerId === c.id && p.conflict,
                            )) ||
                          (filter === "updates" &&
                            inventory.images.some(
                              (i) =>
                                i.id === c.imageId &&
                                i.updateStatus === "update_available",
                            ))),
                    )
                    .map((c) => (
                      <ContainerRow
                        key={c.id}
                        container={c}
                        open={() => open({ kind: "container", container: c })}
                        inspect={() => void inspect("container", c.id)}
                        report={report}
                      />
                    ))}
                  {!inventory.standalone.length && (
                    <Empty>
                      Aucun conteneur autonome. Les services Compose sont
                      regroupés dans leurs projets.
                    </Empty>
                  )}
                </div>
              )}
              {page === "ports" && (
                <>
                  <PortProbe report={report} />
                  <div className="panel">
                    <PortTable
                      ports={inventory.ports.filter(
                        (p) =>
                          matches(
                            `${p.hostPort} ${p.ownerName} ${p.hostIp} ${p.service || ""}`,
                          ) &&
                          (filter === "all" ||
                            (filter === "conflicts"
                              ? !!p.conflict
                              : p.status === filter)),
                      )}
                      onOwner={(port) => {
                        const project = inventory.projects.find(
                          (p) => p.id === port.ownerId,
                        );
                        const container = inventory.standalone.find(
                          (c) => c.id === port.ownerId,
                        );
                        if (project) open({ kind: "project", project });
                        else if (container)
                          open({ kind: "container", container });
                      }}
                      suggest={(port) => {
                        const project = inventory.projects.find(
                          (p) => p.id === port.ownerId,
                        );
                        if (
                          project?.definition &&
                          port.conflict?.suggestedPort
                        ) {
                          open({ kind: "project", project });
                          setSuggested({
                            from: port.hostPort,
                            to: port.conflict.suggestedPort,
                          });
                          setTab("yaml");
                        }
                      }}
                    />
                  </div>
                  <p className="footnote">
                    <ShieldCheck size={14} />
                    Les sondages TCP de l’hôte sont indicatifs. Un port
                    silencieux n’est pas garanti libre sur toutes les
                    interfaces.
                  </p>
                </>
              )}
              {page === "images" && (
                <div className="panel table-scroll">
                  <table>
                    <thead>
                      <tr>
                        <th>Image</th>
                        <th>Taille</th>
                        <th>Conteneurs</th>
                        <th>Mise à jour</th>
                        <th />
                      </tr>
                    </thead>
                    <tbody>
                      {inventory.images
                        .filter((i) => matches(`${i.tags.join(" ")} ${i.id}`))
                        .map((image) => (
                          <tr key={image.id}>
                            <td>
                              <button
                                className="text-button"
                                onClick={() => void inspect("image", image.id)}
                              >
                                {image.tags[0] || "<sans tag>"}
                              </button>
                              <small className="cell-secondary">
                                {image.id.slice(7, 19)}
                              </small>
                            </td>
                            <td>{bytes(image.size)}</td>
                            <td>
                              {image.containers >= 0 ? image.containers : "—"}
                            </td>
                            <td>
                              <Badge value={image.updateStatus} />
                            </td>
                            <td>
                              <div className="row-actions">
                                <button
                                  onClick={() =>
                                    void submit(
                                      `image:${image.id}`,
                                      "check_update",
                                    )
                                  }
                                >
                                  <RefreshCw size={13} />
                                  Vérifier
                                </button>
                                <button
                                  className="icon-button danger"
                                  aria-label="Supprimer l’image"
                                  onClick={() =>
                                    confirmAction(
                                      `image:${image.id}`,
                                      "remove",
                                      image.tags[0] || image.id,
                                    )
                                  }
                                >
                                  <Trash2 size={15} />
                                </button>
                              </div>
                            </td>
                          </tr>
                        ))}
                    </tbody>
                  </table>
                  {!inventory.images.length && (
                    <Empty>Aucune image disponible.</Empty>
                  )}
                </div>
              )}
              {page === "networks" && (
                <div className="panel table-scroll">
                  <table>
                    <thead>
                      <tr>
                        <th>Réseau</th>
                        <th>Driver</th>
                        <th>Scope</th>
                        <th>Conteneurs</th>
                        <th>Actions</th>
                      </tr>
                    </thead>
                    <tbody>
                      {inventory.networks
                        .filter((n) => matches(`${n.name} ${n.driver}`))
                        .map((n) => (
                          <tr key={n.id}>
                            <td>
                              <button
                                className="text-button"
                                onClick={() => void inspect("network", n.id)}
                              >
                                {n.name}
                              </button>
                              {n.internal && (
                                <small className="cell-secondary">
                                  Interne
                                </small>
                              )}
                            </td>
                            <td>{n.driver}</td>
                            <td>{n.scope}</td>
                            <td>{n.containers}</td>
                            <td>
                              <div className="row-actions">
                                <button
                                  onClick={() => networkLink(n.id, "connect")}
                                >
                                  Connecter
                                </button>
                                <button
                                  onClick={() =>
                                    networkLink(n.id, "disconnect")
                                  }
                                >
                                  Déconnecter
                                </button>
                                <button
                                  className="icon-button danger"
                                  aria-label="Supprimer le réseau"
                                  onClick={() =>
                                    confirmAction(
                                      `network:${n.id}`,
                                      "remove",
                                      n.name,
                                    )
                                  }
                                >
                                  <Trash2 size={15} />
                                </button>
                              </div>
                            </td>
                          </tr>
                        ))}
                    </tbody>
                  </table>
                </div>
              )}
              {page === "volumes" && (
                <div className="panel table-scroll">
                  <table>
                    <thead>
                      <tr>
                        <th>Volume</th>
                        <th>Driver</th>
                        <th>Point de montage</th>
                        <th />
                      </tr>
                    </thead>
                    <tbody>
                      {inventory.volumes
                        .filter((v) => matches(`${v.name} ${v.driver}`))
                        .map((v) => (
                          <tr key={v.name}>
                            <td>
                              <button
                                className="text-button"
                                onClick={() => void inspect("volume", v.name)}
                              >
                                {v.name}
                              </button>
                            </td>
                            <td>{v.driver}</td>
                            <td>
                              <code>{v.mountpoint}</code>
                            </td>
                            <td>
                              <button
                                className="icon-button danger"
                                aria-label="Supprimer le volume"
                                onClick={() =>
                                  confirmAction(
                                    `volume:${v.name}`,
                                    "remove",
                                    v.name,
                                  )
                                }
                              >
                                <Trash2 size={15} />
                              </button>
                            </td>
                          </tr>
                        ))}
                    </tbody>
                  </table>
                  {!inventory.volumes.length && <Empty>Aucun volume.</Empty>}
                </div>
              )}
              {page === "activity" && (
                <EngineEvents events={inventory.events || []} />
              )}
              {page === "activity" && (
                <div className="panel">
                  <div className="panel-heading">
                    <h2>Journal des opérations</h2>
                    <span className="count">{jobs.length}</span>
                  </div>
                  {jobs.length ? (
                    jobs.map((job) => (
                      <details className="job-row" key={job.id}>
                        <summary>
                          <div className={`job-icon ${stateTone(job.state)}`}>
                            <Activity size={16} />
                          </div>
                          <div className="grow">
                            <strong>{job.action}</strong>
                            <small>{job.target}</small>
                          </div>
                          <Badge value={job.state} />
                          <time>{time(job.createdAt)}</time>
                          <ChevronRight size={15} />
                        </summary>
                        <div className="job-detail">
                          <div className="progress">
                            <i style={{ width: `${job.progress}%` }} />
                          </div>
                          {job.error && (
                            <pre className="error-text">{job.error}</pre>
                          )}
                          <pre>
                            {job.output.join("\n") ||
                              "En attente de la sortie de l’opération…"}
                          </pre>
                        </div>
                      </details>
                    ))
                  ) : (
                    <Empty>
                      Les opérations et leur résultat apparaîtront ici.
                    </Empty>
                  )}
                </div>
              )}
              {page === "settings" && (
                <div className="settings-grid">
                  <div className="panel">
                    <div className="panel-heading">
                      <h2>Installation locale</h2>
                      <ShieldCheck size={18} />
                    </div>
                    <div className="info-list">
                      <div>
                        <span>Racine des projets</span>
                        <code>{session?.root}</code>
                      </div>
                      <div>
                        <span>Version</span>
                        <strong>{session?.version}</strong>
                      </div>
                      <div>
                        <span>Accès web</span>
                        <strong>{location.origin}</strong>
                      </div>
                      <div>
                        <span>Moteur</span>
                        <Badge
                          value={
                            inventory.engineConnected ? "running" : "stopped"
                          }
                        />
                      </div>
                    </div>
                    <p className="panel-copy">
                      Configurez la racine, le socket et les permissions dans le
                      fichier .env généré par scripts/setup.sh. Montez la racine
                      au même chemin absolu pour préserver les bind mounts.
                    </p>
                  </div>
                  <div className="panel">
                    <div className="panel-heading">
                      <h2>Registres privés</h2>
                    </div>
                    <p className="panel-copy">
                      Montez une configuration Docker dédiée en lecture seule
                      pour authentifier les pulls. Les identifiants restent dans
                      cette configuration.
                    </p>
                    {session?.credentialDiagnostics.length ? (
                      session.credentialDiagnostics.map((d) => (
                        <div className="callout warning" key={d}>
                          {d}
                        </div>
                      ))
                    ) : (
                      <p className="panel-copy green-text">
                        <Check size={15} />
                        Aucun helper natif incompatible détecté.
                      </p>
                    )}
                  </div>
                  <div className="panel">
                    <div className="panel-heading">
                      <h2>À propos de CompMaster</h2>
                    </div>
                    <p className="panel-copy">
                      CompMaster est distribué sous licence GNU GPL v3, sans
                      garantie. L’accès au socket Docker permet d’administrer le
                      moteur et les ressources de l’hôte.
                    </p>
                    <a
                      className="text-button panel-copy"
                      href="https://www.gnu.org/licenses/gpl-3.0.html"
                      target="_blank"
                      rel="noreferrer"
                    >
                      Consulter la licence GNU GPL v3 ↗
                    </a>
                  </div>
                </div>
              )}
            </>
          )}
          <footer>
            <span>
              <span className="tiny-dot" />
              Synchronisation automatique
            </span>
            <span>
              {inventory
                ? `Dernière mise à jour ${time(inventory.generatedAt)}`
                : "Initialisation…"}
            </span>
          </footer>
        </div>
      </main>
      {toast && (
        <div className={`toast ${toast.error ? "error" : ""}`} role="status">
          {toast.error ? <TriangleAlert size={19} /> : <Check size={19} />}
          <span>{toast.text}</span>
          <button
            className="icon-button"
            aria-label="Fermer"
            onClick={() => setToast(null)}
          >
            <X size={16} />
          </button>
        </div>
      )}
      {modal && (
        <div className="modal-backdrop" onClick={() => setModal(null)}>
          <div
            role="dialog"
            aria-modal="true"
            aria-label={modal.title}
            className="modal"
            onClick={(e) => e.stopPropagation()}
          >
            <div className="modal-title">
              <h2>{modal.title}</h2>
              <button
                className="icon-button"
                aria-label="Fermer"
                onClick={() => setModal(null)}
              >
                <X size={19} />
              </button>
            </div>
            {modal.content}
          </div>
        </div>
      )}
    </div>
  );
}

function projectFilter(project: Project, filter: string, inventory: Inventory) {
  if (filter === "all") return true;
  if (filter === "running")
    return project.containers.some((c) => c.state === "running");
  if (filter === "conflicts")
    return inventory.ports.some((p) => p.ownerId === project.id && p.conflict);
  if (filter === "unhealthy")
    return project.containers.some((c) => c.health === "unhealthy");
  if (filter === "updates")
    return project.containers.some((c) =>
      inventory.images.some(
        (i) => i.id === c.imageId && i.updateStatus === "update_available",
      ),
    );
  return project.state === filter;
}

function EngineEvents({ events }: { events: EngineEvent[] }) {
  return (
    <div className="panel">
      <div className="panel-heading">
        <h2>Événements Docker en direct</h2>
        <span className="count">{events.length}</span>
      </div>
      {events.length ? (
        <div className="table-scroll">
          <table>
            <thead>
              <tr>
                <th>Heure</th>
                <th>Ressource</th>
                <th>Action</th>
                <th>Type</th>
              </tr>
            </thead>
            <tbody>
              {events.slice(0, 100).map((event, index) => (
                <tr key={index}>
                  <td>{time(new Date(event.time * 1000).toISOString())}</td>
                  <td>
                    <code>{event.name || event.id?.slice(0, 12)}</code>
                  </td>
                  <td>{event.action}</td>
                  <td>{event.kind}</td>
                </tr>
              ))}
            </tbody>
          </table>
        </div>
      ) : (
        <p className="panel-copy">
          Les nouveaux événements du moteur apparaîtront ici. Les 200 derniers
          événements sont conservés pendant cette session.
        </p>
      )}
    </div>
  );
}
function Metric({
  label,
  value,
  detail,
  icon,
  tone = "",
}: {
  label: string;
  value: number;
  detail: string;
  icon: ReactNode;
  tone?: string;
}) {
  return (
    <div className="metric">
      <div className="metric-label">
        <span>{label}</span>
        <span className={tone ? `${tone}-text` : ""}>{icon}</span>
      </div>
      <div className="metric-value">
        {value}
        <span className={`metric-line ${tone}`} />
      </div>
      <small>{detail}</small>
    </div>
  );
}
function ContainerRow({
  container: c,
  open,
  inspect,
  report,
}: {
  container: Container;
  open: () => void;
  inspect: () => void;
  report: (s: string, error?: boolean) => void;
}) {
  const [stats, setStats] = useState<{
    CPUPerc: string;
    MemUsage: string;
  } | null>(null);
  useEffect(() => {
    if (c.state !== "running") return;
    let active = true;
    const load = () =>
      api<{ CPUPerc: string; MemUsage: string }>(`/containers/${c.id}/stats`)
        .then((s) => {
          if (active) setStats(s);
        })
        .catch(() => {});
    void load();
    const timer = setInterval(load, 10000);
    return () => {
      active = false;
      clearInterval(timer);
    };
  }, [c.id, c.state]);
  return (
    <div className="container-row">
      <div className="container-symbol">
        <Box size={21} />
      </div>
      <button className="container-name" onClick={open}>
        <strong>{c.name}</strong>
        <small>{c.image}</small>
      </button>
      <div className="container-stats">
        {stats ? (
          <>
            <span>
              {stats.CPUPerc}
              <small>CPU</small>
            </span>
            <span>
              {stats.MemUsage.split(" / ")[0]}
              <small>MEM</small>
            </span>
          </>
        ) : (
          <code>{c.shortId}</code>
        )}
      </div>
      <Badge value={c.health === "unhealthy" ? c.health : c.state} />
      <button
        className="icon-button"
        aria-label="Copier l’identifiant"
        onClick={() => {
          void navigator.clipboard
            .writeText(c.id)
            .then(() => report("Identifiant copié"))
            .catch(() => report("Copie impossible", true));
        }}
      >
        <Copy size={14} />
      </button>
      <button onClick={inspect}>Inspecter</button>
    </div>
  );
}
function PortTable({
  ports,
  onOwner,
  suggest,
}: {
  ports: Port[];
  onOwner: (p: Port) => void;
  suggest: (p: Port) => void;
}) {
  if (!ports.length) return <Empty>Aucun port correspondant.</Empty>;
  return (
    <div className="table-scroll">
      <table>
        <thead>
          <tr>
            <th>Port hôte</th>
            <th>Destination</th>
            <th>Propriétaire</th>
            <th>Statut</th>
            <th>Conflit / proposition</th>
          </tr>
        </thead>
        <tbody>
          {ports.map((port) => (
            <tr key={port.id}>
              <td>
                <div className="port-number">
                  <Cable size={15} />
                  <strong>
                    {port.hostPort}
                    {port.hostPortEnd ? `–${port.hostPortEnd}` : ""}
                  </strong>
                  <span>/{port.protocol}</span>
                </div>
                <small className="cell-secondary">
                  {port.hostIp || "0.0.0.0"}
                </small>
              </td>
              <td>
                <code>
                  {port.source === "external"
                    ? "Hôte"
                    : `→ ${port.containerPort}`}
                </code>
              </td>
              <td>
                <button className="text-button" onClick={() => onOwner(port)}>
                  {port.ownerName}
                </button>
                <small className="cell-secondary">
                  {port.service || "autonome"}
                </small>
              </td>
              <td>
                <Badge value={port.status} />
              </td>
              <td>
                {port.conflict ? (
                  <div className="port-conflict">
                    <span>
                      <TriangleAlert size={13} />
                      {port.conflict.message}
                    </span>
                    {port.conflict.suggestedPort && (
                      <button onClick={() => suggest(port)}>
                        Proposer :{port.conflict.suggestedPort}
                        <ArrowRight size={12} />
                      </button>
                    )}
                  </div>
                ) : (
                  <span className="muted">—</span>
                )}
              </td>
            </tr>
          ))}
        </tbody>
      </table>
    </div>
  );
}
function PortProbe({
  report,
}: {
  report: (s: string, error?: boolean) => void;
}) {
  const [result, setResult] = useState<
    { port: number; status: string; suggestedPort: number | null }[]
  >([]);
  const [busy, setBusy] = useState(false);
  return (
    <div className="probe-panel">
      <form
        onSubmit={async (e) => {
          e.preventDefault();
          const data = new FormData(e.currentTarget);
          const ports = String(data.get("ports"))
            .split(/[,\s]+/)
            .map(Number)
            .filter((p) => Number.isInteger(p) && p > 0 && p < 65536);
          setBusy(true);
          try {
            const response = await api<{ results: typeof result }>(
              "/ports/probe",
              "POST",
              { ports },
            );
            setResult(response.results);
          } catch (e) {
            report((e as Error).message, true);
          } finally {
            setBusy(false);
          }
        }}
      >
        <div>
          <h3>Sonder les ports de l’hôte</h3>
          <p>Détecter aussi un service TCP en dehors de Docker.</p>
        </div>
        <input
          name="ports"
          aria-label="Ports à sonder"
          placeholder="8000, 8080, 5432…"
          required
        />
        <button disabled={busy}>
          <Search size={15} />
          {busy ? "Sondage…" : "Vérifier"}
        </button>
      </form>
      {result.length > 0 && (
        <div className="probe-results">
          {result.map((r) => (
            <div key={r.port}>
              <strong>:{r.port}</strong>
              <Badge value={r.status} />
              {r.suggestedPort && (
                <small>Alternative probable :{r.suggestedPort}</small>
              )}
            </div>
          ))}
        </div>
      )}
    </div>
  );
}
function TerminalSelector({ containers }: { containers: Container[] }) {
  const [id, setId] = useState(
    containers.find((c) => c.state === "running")?.id || "",
  );
  return (
    <>
      <div className="toolbar">
        <TerminalSquare size={17} />
        <select
          aria-label="Conteneur du terminal"
          value={id}
          onChange={(e) => setId(e.target.value)}
        >
          <option value="">Choisir un conteneur actif</option>
          {containers
            .filter((c) => c.state === "running")
            .map((c) => (
              <option key={c.id} value={c.id}>
                {c.name}
              </option>
            ))}
        </select>
      </div>
      {id ? (
        <Terminal key={id} id={id} />
      ) : (
        <Empty>Démarrez un conteneur pour ouvrir un terminal.</Empty>
      )}
    </>
  );
}
function Replacement({
  id,
  report,
  onSubmit,
}: {
  id: string;
  report: (s: string, error?: boolean) => void;
  onSubmit: (options: object) => void;
}) {
  const [original, setOriginal] = useState("");
  const [content, setContent] = useState("");
  const [preview, setPreview] = useState(false);
  useEffect(() => {
    api<Record<string, Json>>(`/inspect/container/${id}`)
      .then((data) => {
        const config = data.Config as Record<string, Json>,
          host = data.HostConfig as Record<string, Json>;
        const value = JSON.stringify(
          {
            config: Object.fromEntries(
              [
                "Image",
                "Cmd",
                "Entrypoint",
                "Env",
                "Healthcheck",
                "WorkingDir",
                "User",
              ]
                .filter((k) => k in config)
                .map((k) => [k, config[k]]),
            ),
            hostConfig: Object.fromEntries(
              [
                "PortBindings",
                "Binds",
                "Mounts",
                "RestartPolicy",
                "Memory",
                "NanoCpus",
              ]
                .filter((k) => k in host)
                .map((k) => [k, host[k]]),
            ),
          },
          null,
          2,
        );
        setOriginal(value);
        setContent(value);
      })
      .catch((e) => report(e.message, true));
  }, [id, report]);
  return (
    <div>
      <p>
        L’ancien conteneur sera conservé et restauré si le nouveau échoue. Les
        volumes partagés ne sont pas sauvegardés par cette opération.
      </p>
      {preview ? (
        <div className="diff-columns">
          <div>
            <small>Avant</small>
            <pre>{original}</pre>
          </div>
          <div>
            <small>Après</small>
            <pre>{content}</pre>
          </div>
        </div>
      ) : (
        <textarea
          className="replacement-editor"
          aria-label="Configuration du remplacement"
          value={content}
          onChange={(e) => setContent(e.target.value)}
          rows={20}
        />
      )}
      <div className="modal-actions">
        <button
          onClick={() => {
            try {
              JSON.parse(content);
              setPreview(!preview);
            } catch (e) {
              report((e as Error).message, true);
            }
          }}
        >
          {preview ? "Modifier" : "Prévisualiser le diff"}
        </button>
        <button
          className="danger"
          disabled={!preview || content === original}
          onClick={() => {
            try {
              onSubmit(JSON.parse(content));
            } catch (e) {
              report((e as Error).message, true);
            }
          }}
        >
          Confirmer le remplacement
        </button>
      </div>
    </div>
  );
}
