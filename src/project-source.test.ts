import { describe, expect, it } from "vitest";
import fixtures from "../fixtures/cloud-api.json";
import { cloneName, projectSource, sameProjectRemote } from "./project-source";

// The desktop test `sources_match_cloud_contract_and_names_cannot_escape_destination` reads the same
// fixture, so the browser mock and the native clone accept the same sources.
describe("project sources", () => {
  it("follow the shared clone source contract", () => {
    for (const source of fixtures.project_sources.valid) expect(projectSource(source), source).not.toBeNull();
    for (const source of fixtures.project_sources.invalid) expect(projectSource(source), source).toBeNull();
  });

  it("name the clone folder after the last source segment", () => {
    expect(cloneName("team/app")).toBe("app");
    expect(cloneName("https://github.com/team/app.git")).toBe("app");
    expect(cloneName("https://github.com/team/app/")).toBe("app");
    expect(cloneName("git@github.com:team/app.git")).toBe("app");
    for (const source of ["github.com/team/.git", "", "team/..", "team/-option"]) expect(cloneName(source), source).toBeNull();
  });

  it("compare remotes across shorthand, https and .git forms", () => {
    expect(sameProjectRemote("team/app", "https://github.com/team/app.git")).toBe(true);
    expect(sameProjectRemote("team/app", "team/other")).toBe(false);
    expect(sameProjectRemote("team/app", "")).toBe(false);
  });
});
