import { useEffect, useRef, useState } from "react";
import { Download, Pause, Play } from "lucide-react";
import { download } from "./api";
import type { Container } from "./types";

type Line = { source: string; stream: string; line: string };
export default function Logs({ containers }: { containers: Container[] }) {
  const [lines, setLines] = useState<Line[]>([]);
  const [service, setService] = useState("all");
  const [filter, setFilter] = useState("");
  const [follow, setFollow] = useState(true);
  const end = useRef<HTMLDivElement>(null);
  const ids = containers.map((c) => c.id).join(",");
  useEffect(() => {
    setLines([]);
    const sources = containers.map((container) => {
      const source = new EventSource(`/api/v1/containers/${container.id}/logs`);
      source.addEventListener("log", (event) => {
        const data = JSON.parse(event.data) as { stream: string; line: string };
        setLines((lines) => [
          ...lines.slice(-4999),
          { ...data, source: container.name },
        ]);
      });
      source.addEventListener("end", () => source.close());
      return source;
    });
    return () => sources.forEach((s) => s.close());
    // Container identity is the subscription key.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [ids]);
  useEffect(() => {
    if (follow) end.current?.scrollIntoView({ block: "nearest" });
  }, [lines, follow]);
  const visible = lines.filter(
    (l) =>
      (service === "all" || l.source === service) &&
      l.line.toLowerCase().includes(filter.toLowerCase()),
  );
  return (
    <>
      <div className="toolbar">
        <select
          aria-label="Service"
          value={service}
          onChange={(e) => setService(e.target.value)}
        >
          <option value="all">Tous les services</option>
          {containers.map((c) => (
            <option key={c.id}>{c.name}</option>
          ))}
        </select>
        <input
          aria-label="Filtrer les logs"
          value={filter}
          onChange={(e) => setFilter(e.target.value)}
          placeholder="Filtrer les logs…"
        />
        <button className="ghost" onClick={() => setFollow(!follow)}>
          {follow ? <Pause size={16} /> : <Play size={16} />}
        </button>
        <button
          className="ghost"
          onClick={() =>
            download(
              "compmaster.log",
              visible.map((l) => `[${l.source}] ${l.line}`).join("\n"),
            )
          }
        >
          <Download size={16} />
          Exporter
        </button>
      </div>
      <div className="logs" aria-live="off">
        {visible.length ? (
          visible.map((line, i) => (
            <div
              className={line.stream === "stderr" ? "log-error" : ""}
              key={i}
            >
              <span>{line.source}</span> {line.line}
            </div>
          ))
        ) : (
          <div className="muted">En attente de logs…</div>
        )}
        <div ref={end} />
      </div>
      <small className="muted">
        Affichage limité aux 5 000 dernières lignes reçues.
      </small>
    </>
  );
}
