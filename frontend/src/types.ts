export type Json =
  null | boolean | number | string | Json[] | { [key: string]: Json };
export type Container = {
  id: string;
  shortId: string;
  name: string;
  image: string;
  imageId: string;
  state: string;
  status: string;
  health: string | null;
  created: number;
  labels: Record<string, string>;
  ports: {
    hostIp: string;
    hostPort: number;
    containerPort: number;
    protocol: string;
  }[];
  mounts: {
    kind: string;
    source: string;
    destination: string;
    readOnly: boolean;
  }[];
  networks: string[];
  networkAddresses?: Record<string, string[]>;
};
export type Definition = {
  id: string;
  path: string;
  files: string[];
  relativePath: string;
  projectName: string;
  services: string[];
  valid: boolean;
  revision: string;
  diagnostics: { level: string; message: string }[];
};
export type Project = {
  id: string;
  name: string;
  state: string;
  definition: Definition | null;
  containers: Container[];
  services: string[];
  favorite: boolean;
  diagnostics: { level: string; message: string }[];
};
export type Port = {
  id: string;
  hostIp: string;
  hostPort: number;
  hostPortEnd?: number | null;
  containerPort: number;
  protocol: string;
  ownerId: string;
  ownerName: string;
  service: string | null;
  source: string;
  status: string;
  conflict: {
    message: string;
    kind: string;
    suggestedPort: number | null;
  } | null;
};
export type Image = {
  id: string;
  tags: string[];
  size: number;
  created: number;
  containers: number;
  updateStatus: string;
};
export type Network = {
  id: string;
  name: string;
  driver: string;
  scope: string;
  internal: boolean;
  containers: number;
};
export type Volume = {
  name: string;
  driver: string;
  mountpoint: string;
  labels: Record<string, string>;
};
export type Inventory = {
  events?: EngineEvent[];
  generatedAt: string;
  engineConnected: boolean;
  engineError: string | null;
  projects: Project[];
  standalone: Container[];
  images: Image[];
  networks: Network[];
  volumes: Volume[];
  ports: Port[];
  summary: {
    projects: number;
    running: number;
    standalone: number;
    unhealthy: number;
    portConflicts: number;
  };
};
export type EngineEvent = {
  kind: string;
  action: string;
  id: string;
  name: string | null;
  time: number;
};
export type Job = {
  id: string;
  target: string;
  action: string;
  state: string;
  progress: number;
  output: string[];
  error: string | null;
  createdAt: string;
  finishedAt: string | null;
};
export type Session = {
  csrfToken: string;
  root: string;
  version: string;
  credentialDiagnostics: string[];
};
