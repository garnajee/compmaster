import { useEffect, useRef, useState } from "react";
import { Terminal as XTerminal } from "@xterm/xterm";
import { FitAddon } from "@xterm/addon-fit";
import "@xterm/xterm/css/xterm.css";
import { csrfToken } from "./api";

export default function Terminal({ id }: { id: string }) {
  const ref = useRef<HTMLDivElement>(null);
  const [command, setCommand] = useState("/bin/sh");
  const [active, setActive] = useState<string | null>(null);
  useEffect(() => {
    if (!active || !ref.current) return;
    const terminal = new XTerminal({
      cursorBlink: true,
      fontSize: 13,
      fontFamily: "ui-monospace, monospace",
      theme: {
        background: "#080d13",
        foreground: "#c4d2e5",
        cursor: "#7ce1c5",
      },
    });
    const fit = new FitAddon();
    terminal.loadAddon(fit);
    terminal.open(ref.current);
    fit.fit();
    const socket = new WebSocket(
      `${location.protocol === "https:" ? "wss" : "ws"}://${location.host}/ws/terminal/${encodeURIComponent(id)}?token=${encodeURIComponent(csrfToken())}&command=${encodeURIComponent(active)}`,
    );
    socket.binaryType = "arraybuffer";
    const resize = () => {
      fit.fit();
      if (socket.readyState === WebSocket.OPEN)
        socket.send(
          JSON.stringify({ cols: terminal.cols, rows: terminal.rows }),
        );
    };
    socket.onopen = resize;
    socket.onmessage = (event) =>
      terminal.write(
        typeof event.data === "string"
          ? event.data
          : new Uint8Array(event.data),
      );
    socket.onerror = () =>
      terminal.write(
        "\r\nConnexion impossible. Vérifiez le conteneur et la commande.\r\n",
      );
    socket.onclose = () => terminal.write("\r\nSession terminée.\r\n");
    const disposable = terminal.onData((data) => {
      if (socket.readyState === WebSocket.OPEN)
        socket.send(new TextEncoder().encode(data));
    });
    const observer = new ResizeObserver(resize);
    observer.observe(ref.current);
    return () => {
      disposable.dispose();
      observer.disconnect();
      socket.close();
      terminal.dispose();
    };
  }, [id, active]);
  return (
    <div className="terminal-panel">
      <div className="toolbar">
        <input
          aria-label="Commande du terminal"
          value={command}
          onChange={(e) => setCommand(e.target.value)}
          placeholder="/bin/sh"
        />
        <button onClick={() => setActive(command)}>Ouvrir</button>
        <button className="ghost" onClick={() => setActive(null)}>
          Déconnecter
        </button>
      </div>
      <p className="muted">
        Exécution dans le conteneur. L’image doit contenir cette commande.
      </p>
      <div className="terminal" ref={ref} />
    </div>
  );
}
