import { mount } from "@vue/test-utils";
import { describe, expect, it } from "vitest";

import type { ProjectFilesLibraryDialogsBindings } from "../composables/project-library/use-project-files-dialogs";
import ProjectFilesLibraryDialogs from "./ProjectFilesLibraryDialogs.vue";

const CreateFolderStub = {
  name: "LibraryCreateFolderDialog",
  props: ["open", "busy", "parentName"],
  emits: ["cancel", "confirm"],
  template: "<div class=\"create-folder-stub\" />",
};

const CreateTextFileStub = {
  name: "LibraryCreateTextFileDialog",
  props: ["open", "busy", "parentName"],
  emits: ["cancel", "confirm"],
  template: "<div class=\"create-text-file-stub\" />",
};

const MoveStub = {
  name: "LibraryMoveDialog",
  props: ["open", "busy", "title", "description", "currentFolderId", "options"],
  emits: ["cancel", "confirm"],
  template: "<div class=\"move-stub\" />",
};

const SourceFolderStub = {
  name: "ProjectSourceFolderDialog",
  props: ["open", "busy", "folderName", "folderNameReadonly", "title", "value"],
  emits: ["cancel", "confirm", "update:value"],
  template: "<div class=\"source-folder-stub\" />",
};

type Bindings = ProjectFilesLibraryDialogsBindings;

function createBindings(): Bindings {
  return {
    createFolder: { open: true, busy: true, parentName: "reports" },
    createTextFile: { open: true, busy: false, parentName: "notes" },
    move: {
      open: true,
      busy: true,
      title: "Move file",
      description: "Move doc.txt",
      currentFolderId: "folder-1",
      options: [
        { label: "Root", value: null },
        { label: "Reports", value: "folder-2" },
      ],
    },
    sourceFolder: {
      open: true,
      busy: true,
      folderName: "analytics",
      folderNameReadonly: true,
      title: "Edit source config",
      value: "{ \"source_key\": \"analytics\" }",
    },
  };
}

function mountDialogs(overrides: Partial<Bindings> = {}) {
  return mount(ProjectFilesLibraryDialogs, {
    props: { ...createBindings(), ...overrides },
    global: {
      stubs: {
        LibraryCreateFolderDialog: CreateFolderStub,
        LibraryCreateTextFileDialog: CreateTextFileStub,
        LibraryMoveDialog: MoveStub,
        ProjectSourceFolderDialog: SourceFolderStub,
      },
    },
  });
}

describe("ProjectFilesLibraryDialogs", () => {
  it("passes each dialog's bound props through unchanged", () => {
    const wrapper = mountDialogs();

    const createFolder = wrapper.getComponent({ name: "LibraryCreateFolderDialog" });
    expect(createFolder.props("open")).toBe(true);
    expect(createFolder.props("busy")).toBe(true);
    expect(createFolder.props("parentName")).toBe("reports");

    const createTextFile = wrapper.getComponent({ name: "LibraryCreateTextFileDialog" });
    expect(createTextFile.props("open")).toBe(true);
    expect(createTextFile.props("busy")).toBe(false);
    expect(createTextFile.props("parentName")).toBe("notes");

    const move = wrapper.getComponent({ name: "LibraryMoveDialog" });
    expect(move.props("open")).toBe(true);
    expect(move.props("busy")).toBe(true);
    expect(move.props("title")).toBe("Move file");
    expect(move.props("description")).toBe("Move doc.txt");
    expect(move.props("currentFolderId")).toBe("folder-1");
    expect(move.props("options")).toEqual([
      { label: "Root", value: null },
      { label: "Reports", value: "folder-2" },
    ]);

    const sourceFolder = wrapper.getComponent({ name: "ProjectSourceFolderDialog" });
    expect(sourceFolder.props("open")).toBe(true);
    expect(sourceFolder.props("busy")).toBe(true);
    expect(sourceFolder.props("folderName")).toBe("analytics");
    expect(sourceFolder.props("folderNameReadonly")).toBe(true);
    expect(sourceFolder.props("title")).toBe("Edit source config");
    expect(sourceFolder.props("value")).toBe("{ \"source_key\": \"analytics\" }");

    wrapper.unmount();
  });

  it("forwards the create-folder confirm name and cancel", async () => {
    const wrapper = mountDialogs();
    const createFolder = wrapper.getComponent({ name: "LibraryCreateFolderDialog" });

    await createFolder.vm.$emit("confirm", "quarterly");
    await createFolder.vm.$emit("cancel");

    expect(wrapper.emitted("create-folder-confirm")).toEqual([["quarterly"]]);
    expect(wrapper.emitted("create-folder-cancel")).toEqual([[]]);
    wrapper.unmount();
  });

  it("forwards the create-text-file confirm payload and cancel", async () => {
    const wrapper = mountDialogs();
    const createTextFile = wrapper.getComponent({ name: "LibraryCreateTextFileDialog" });

    await createTextFile.vm.$emit("confirm", { title: "notes", content: "body" });
    await createTextFile.vm.$emit("cancel");

    expect(wrapper.emitted("create-text-file-confirm")).toEqual([[{ title: "notes", content: "body" }]]);
    expect(wrapper.emitted("create-text-file-cancel")).toEqual([[]]);
    wrapper.unmount();
  });

  it("forwards the move target folder and cancel", async () => {
    const wrapper = mountDialogs();
    const move = wrapper.getComponent({ name: "LibraryMoveDialog" });

    await move.vm.$emit("confirm", "folder-2");
    await move.vm.$emit("confirm", null);
    await move.vm.$emit("cancel");

    expect(wrapper.emitted("move-confirm")).toEqual([["folder-2"], [null]]);
    expect(wrapper.emitted("move-cancel")).toEqual([[]]);
    wrapper.unmount();
  });

  it("forwards the source-folder confirm payload, cancel and value update", async () => {
    const wrapper = mountDialogs();
    const sourceFolder = wrapper.getComponent({ name: "ProjectSourceFolderDialog" });

    await sourceFolder.vm.$emit("confirm", { folderName: "analytics", value: "{}" });
    await sourceFolder.vm.$emit("cancel");
    await sourceFolder.vm.$emit("update:value", "{\"next\":true}");

    expect(wrapper.emitted("source-folder-confirm")).toEqual([[{ folderName: "analytics", value: "{}" }]]);
    expect(wrapper.emitted("source-folder-cancel")).toEqual([[]]);
    expect(wrapper.emitted("source-folder-update-value")).toEqual([["{\"next\":true}"]]);
    wrapper.unmount();
  });
});
