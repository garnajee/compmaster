import { test, expect, type Page } from "@playwright/test";

const container = {
  id: "a".repeat(64),
  shortId: "a".repeat(12),
  name: "media-web-1",
  image: "nginx:alpine",
  imageId: "sha256:123",
  state: "running",
  status: "Up",
  health: "healthy",
  created: 1,
  labels: { "com.docker.compose.service": "web" },
  ports: [
    { hostIp: "0.0.0.0", hostPort: 8000, containerPort: 80, protocol: "tcp" },
  ],
  mounts: [],
  networks: ["media_default"],
};
const definition = {
  id: "cmp_media",
  path: "/projects/media/custom.yml",
  files: ["/projects/media/custom.yml"],
  relativePath: "media/custom.yml",
  projectName: "media",
  services: ["web"],
  valid: true,
  revision: "abc",
  diagnostics: [],
};
const projects = [
  {
    id: "cmp_media",
    name: "media",
    state: "running",
    definition,
    containers: [container],
    services: ["web"],
    favorite: false,
    diagnostics: [],
  },
  {
    id: "cmp_backup",
    name: "backup",
    state: "discovered",
    definition: {
      ...definition,
      id: "cmp_backup",
      relativePath: "backups/jobs.yaml",
      projectName: "backup",
    },
    containers: [],
    services: ["backup"],
    favorite: false,
    diagnostics: [],
  },
];
const inventory = {
  generatedAt: new Date().toISOString(),
  engineConnected: true,
  engineError: null,
  projects,
  standalone: [],
  images: [],
  networks: [],
  volumes: [],
  ports: [
    {
      id: "port",
      hostIp: "0.0.0.0",
      hostPort: 8000,
      containerPort: 80,
      protocol: "tcp",
      ownerId: "cmp_media",
      ownerName: "media",
      service: "web",
      source: "runtime",
      status: "docker_occupied",
      conflict: {
        message: "Port partagé avec backup",
        kind: "potential",
        suggestedPort: 8001,
      },
    },
  ],
  summary: {
    projects: 2,
    running: 1,
    standalone: 0,
    unhealthy: 0,
    portConflicts: 1,
  },
};
const yaml =
  'name: media\nservices:\n  web:\n    image: nginx:alpine\n    ports:\n      - "8000:80" # preserve me\n';

async function prepare(page: Page) {
  await page.route("**/api/v1/**", async (route) => {
    const path = new URL(route.request().url()).pathname;
    let body: unknown = {};
    if (path.endsWith("/session"))
      body = {
        csrfToken: "test-token",
        root: "/projects",
        version: "0.1.0",
        credentialDiagnostics: [],
      };
    else if (path.endsWith("/inventory")) body = inventory;
    else if (path.endsWith("/jobs")) body = [];
    else if (path.endsWith("/events")) {
      await route.fulfill({
        contentType: "text/event-stream",
        body: ": test\n\n",
      });
      return;
    } else if (path.endsWith("/file"))
      body = {
        content: yaml,
        revision: "abc",
        path: definition.path,
        files: definition.files,
      };
    else if (path.endsWith("/stats"))
      body = { CPUPerc: "0.5%", MemUsage: "12 MiB / 1 GiB" };
    await route.fulfill({ json: body });
  });
  await page.goto("/");
  await expect(
    page.getByRole("heading", { name: "Vos projets, en un regard." }),
  ).toBeVisible();
}
async function navigate(page: Page, label: string) {
  if (await page.getByRole("button", { name: "Menu", exact: true }).isVisible())
    await page.getByRole("button", { name: "Menu", exact: true }).click();
  await page.getByRole("button", { name: label, exact: true }).first().click();
}

test("discovery, search, filter and responsive layout", async ({ page }) => {
  await prepare(page);
  await expect(page.locator(".project-card")).toHaveCount(2);
  await page
    .getByRole("textbox", { name: "Rechercher", exact: true })
    .fill("backup");
  await expect(page.locator(".project-card")).toHaveCount(1);
  await page.getByRole("textbox", { name: "Rechercher", exact: true }).fill("");
  await page.getByLabel("Filtrer par état").selectOption("running");
  await expect(page.locator(".project-card")).toHaveCount(1);
  expect(
    await page.evaluate(
      () => document.documentElement.scrollWidth <= innerWidth,
    ),
  ).toBeTruthy();
  await page.screenshot({
    path: `test-results/dashboard-${test.info().project.name}.png`,
    fullPage: true,
  });
});

test("ports link to source and proposed YAML preserves comments", async ({
  page,
}) => {
  await prepare(page);
  await navigate(page, "Ports");
  await expect(
    page.getByRole("heading", { name: "Cartographie des ports" }),
  ).toBeVisible();
  await page.getByRole("button", { name: "Proposer :8001" }).click();
  await expect(page.getByText("Proposition : port 8000 → 8001")).toBeVisible();
  await page.getByRole("button", { name: "Préparer le diff" }).click();
  await expect(
    page.getByRole("heading", { name: "Modifications à confirmer" }),
  ).toBeVisible();
  await expect(page.locator(".diff-columns > div").last()).toContainText(
    '"8001:80" # preserve me',
  );
  expect(
    await page.evaluate(
      () => document.documentElement.scrollWidth <= innerWidth,
    ),
  ).toBeTruthy();
});

test("destructive actions require confirmation and CSRF", async ({ page }) => {
  await prepare(page);
  await page.locator(".card-link").filter({ hasText: "media" }).click();
  await page.getByRole("button", { name: "Down", exact: true }).click();
  await expect(page.getByRole("dialog")).toBeVisible();
  await page.getByRole("button", { name: "Annuler", exact: true }).click();
  let mutations = 0;
  await page.route("**/api/v1/jobs", async (route) => {
    if (route.request().method() === "POST") {
      mutations++;
      expect(route.request().headers()["x-csrf-token"]).toBe("test-token");
      expect(route.request().postDataJSON().options.confirmed).toBe(true);
    }
    await route.fulfill({
      json: { id: "job", target: "cmp_media", action: "down", state: "queued" },
    });
  });
  expect(mutations).toBe(0);
  await page.getByRole("button", { name: "Down", exact: true }).click();
  await page.getByRole("button", { name: "Confirmer", exact: true }).click();
  await expect.poll(() => mutations).toBe(1);
});

test("invalid YAML is rejected in the browser and stale revisions remain unsaved", async ({
  page,
}) => {
  await prepare(page);
  await page.locator(".card-link").filter({ hasText: "media" }).click();
  await page.getByRole("button", { name: "Compose YAML", exact: true }).click();
  const editor = page.locator(".cm-content");
  await expect(editor).toBeVisible();
  await editor.fill("services: [broken");
  let validations = 0;
  await page.route("**/api/v1/projects/*/validate*", async (route) => {
    validations++;
    await route.fulfill({ json: { valid: true } });
  });
  await page.getByRole("button", { name: "Valider", exact: true }).click();
  await expect(page.locator(".toast.error")).toBeVisible();
  expect(validations).toBe(0);
  await editor.fill(yaml + "# new revision\n");
  await page.route("**/api/v1/projects/*/file*", async (route) => {
    if (route.request().method() === "PUT")
      await route.fulfill({
        status: 409,
        json: {
          error: "Le fichier a changé. Rechargez-le avant d’enregistrer.",
        },
      });
    else await route.fallback();
  });
  await page.getByRole("button", { name: "Enregistrer", exact: true }).click();
  await page
    .getByRole("button", { name: "Valider et enregistrer", exact: true })
    .click();
  await expect(page.locator(".toast.error")).toContainText(
    "Le fichier a changé",
  );
  await expect(page.getByText("Modifications non enregistrées")).toBeVisible();
});
