import { useEffect, useState } from "react";
import CodeMirror from "@uiw/react-codemirror";
import { yaml } from "@codemirror/lang-yaml";
import { Check, Save, RotateCcw, FileDiff } from "lucide-react";
import { api } from "./api";
import { parseDocument } from "yaml";

type FileData = {
  content: string;
  revision: string;
  path: string;
  files: string[];
};
export default function Editor({
  id,
  suggested,
  report,
  saved,
}: {
  id: string;
  suggested?: { from: number; to: number };
  report: (message: string, error?: boolean) => void;
  saved: () => void;
}) {
  const [file, setFile] = useState<FileData | null>(null);
  const [content, setContent] = useState("");
  const [busy, setBusy] = useState(false);
  const [preview, setPreview] = useState(false);
  const [index, setIndex] = useState(0);
  const reload = () =>
    api<FileData>(`/projects/${encodeURIComponent(id)}/file?index=${index}`)
      .then((data) => {
        setFile(data);
        setContent(data.content);
        setPreview(false);
      })
      .catch((e) => report(e.message, true));
  useEffect(() => {
    void reload();
  }, [id, index]); // eslint-disable-line react-hooks/exhaustive-deps
  const validate = async (save = false) => {
    setBusy(true);
    try {
      if (!file) return;
      const document = parseDocument(content, { uniqueKeys: true });
      if (document.errors.length)
        throw new Error(document.errors.map((e) => e.message).join("\n"));
      const data = await api<{ revision: string }>(
        `/projects/${encodeURIComponent(id)}/${save ? "file" : "validate"}?index=${index}`,
        save ? "PUT" : "POST",
        { content, revision: file.revision },
      );
      if (save) {
        setFile({ ...file, content, revision: data.revision });
        setPreview(false);
        saved();
      }
      report(
        save
          ? "Fichier validé et enregistré. Sauvegarde conservée."
          : "Syntaxe YAML et configuration Compose valides.",
      );
    } catch (error) {
      report((error as Error).message, true);
    } finally {
      setBusy(false);
    }
  };
  const suggest = () => {
    if (!suggested || !file) return;
    // Only touch an unambiguous quoted short-form port on a single line; preserve comments and formatting.
    const escaped = String(suggested.from);
    const pattern = new RegExp(
      `(^[ \\t]*-[ \\t]*["'](?:[0-9a-fA-F.:\\[\\]]+:)?)(` +
        escaped +
        `)(:[0-9]+(?:/(?:tcp|udp))?["'][ \\t]*(?:#.*)?$)`,
      "gm",
    );
    let count = 0;
    const candidate = content.replace(
      pattern,
      (_all, a: string, _port: string, b: string) => {
        count++;
        return `${a}${suggested.to}${b}`;
      },
    );
    if (count === 1) {
      setContent(candidate);
      setPreview(true);
      report("Proposition prête. Vérifiez le diff avant enregistrement.");
    } else {
      report(
        `Remplacement ambigu. Recherchez le port ${suggested.from} et modifiez-le dans l’éditeur.`,
        true,
      );
    }
  };
  if (!file) return <div className="empty">Chargement du fichier…</div>;
  const dirty = content !== file.content;
  return (
    <div className="editor">
      {file.files.length > 1 && (
        <div className="toolbar">
          <label htmlFor="compose-file">Fichier Compose</label>
          <select
            id="compose-file"
            value={index}
            disabled={dirty}
            onChange={(e) => setIndex(Number(e.target.value))}
          >
            {file.files.map((path, i) => (
              <option key={path} value={i}>
                {i + 1}. {path}
              </option>
            ))}
          </select>
        </div>
      )}
      <div className="toolbar">
        <code className="grow truncate">{file.path}</code>
        <button className="ghost" onClick={reload}>
          <RotateCcw size={15} />
          Recharger
        </button>
        <button disabled={busy} onClick={() => void validate()}>
          <Check size={15} />
          Valider
        </button>
        <button disabled={!dirty} onClick={() => setPreview(!preview)}>
          <FileDiff size={15} />
          Diff
        </button>
        <button
          className="primary"
          disabled={!dirty || busy}
          onClick={() => setPreview(true)}
        >
          <Save size={15} />
          Enregistrer
        </button>
      </div>
      {suggested && (
        <div className="callout">
          <span>
            Proposition : port {suggested.from} → {suggested.to}
          </span>
          <button onClick={suggest}>Préparer le diff</button>
        </div>
      )}
      {preview && (
        <div className="diff">
          <h3>Modifications à confirmer</h3>
          <div className="diff-columns">
            <div>
              <small>Version enregistrée</small>
              <pre>{file.content}</pre>
            </div>
            <div>
              <small>Nouvelle version</small>
              <pre>{content}</pre>
            </div>
          </div>
          <button
            className="primary"
            disabled={busy || !dirty}
            onClick={() => void validate(true)}
          >
            Valider et enregistrer
          </button>
          <button className="ghost" onClick={() => setPreview(false)}>
            Retour à l’éditeur
          </button>
        </div>
      )}
      <CodeMirror
        value={content}
        height="480px"
        theme="dark"
        extensions={[yaml()]}
        onChange={setContent}
        basicSetup={{ lineNumbers: true, foldGutter: true }}
      />
      <p className="muted">
        {dirty
          ? "Modifications non enregistrées"
          : `Révision ${file.revision.slice(0, 12)}`}{" "}
        · Validation Compose obligatoire avant chaque enregistrement.
      </p>
    </div>
  );
}
