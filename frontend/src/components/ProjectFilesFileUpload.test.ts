import { mount } from "@vue/test-utils";
import { defineComponent, h, nextTick, ref } from "vue";
import { describe, expect, it, vi } from "vitest";

import ProjectFilesFileUpload, { type ProjectFilesFileUploadHandle } from "./ProjectFilesFileUpload.vue";

const FileUploadStub = defineComponent({
  name: "FileUpload",
  props: ["modelValue", "multiple", "preview", "dropzone", "accept"],
  emits: ["update:modelValue"],
  setup(_, { expose }) {
    const inputRef = ref<HTMLInputElement | null>(null);
    expose({ inputRef });
    return () => h("input", { ref: inputRef, class: "file-upload-stub", type: "file" });
  },
});

function mountUpload() {
  return mount(ProjectFilesFileUpload, {
    global: {
      stubs: { FileUpload: FileUploadStub },
    },
  });
}

describe("ProjectFilesFileUpload", () => {
  it("clicks the hidden file input through the exposed trigger", () => {
    const wrapper = mountUpload();
    const input = wrapper.get<HTMLInputElement>("input.file-upload-stub");
    const click = vi.spyOn(input.element, "click");

    (wrapper.vm as unknown as ProjectFilesFileUploadHandle).trigger();

    expect(click).toHaveBeenCalledTimes(1);
    wrapper.unmount();
  });

  it("emits the selected files and resets the upload model", async () => {
    const wrapper = mountUpload();
    const fileUpload = wrapper.getComponent({ name: "FileUpload" });
    const files = [new File(["alpha"], "alpha.txt"), new File(["beta"], "beta.txt")];

    fileUpload.vm.$emit("update:modelValue", files);
    await nextTick();

    expect(wrapper.emitted("select")).toEqual([[{ files }]]);
    expect(fileUpload.props("modelValue")).toBeNull();
    wrapper.unmount();
  });

  it("keeps a selection without files out of the select event", async () => {
    const wrapper = mountUpload();
    const fileUpload = wrapper.getComponent({ name: "FileUpload" });

    fileUpload.vm.$emit("update:modelValue", []);
    await nextTick();

    expect(wrapper.emitted("select")).toBeUndefined();
    expect(fileUpload.props("modelValue")).toEqual([]);
    wrapper.unmount();
  });
});
