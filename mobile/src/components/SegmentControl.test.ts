import assert from "node:assert/strict";
import test from "node:test";
import { createElement } from "react";
import type { SegmentControlProps } from "./SegmentControl";
import {
  renderSegmentControl,
  setupSegmentControl,
  type SegmentControlElement,
} from "./SegmentControl.testHarness";

function nodes(root: SegmentControlElement): SegmentControlElement[] {
  if (typeof root.type === "function") return nodes(root.type(root.props));
  return [
    root,
    ...[root.props.children]
      .flat(Infinity)
      .flatMap((child) =>
        child && typeof child === "object" && "props" in child
          ? nodes(child)
          : [],
      ),
  ];
}

function text(root: SegmentControlElement): string {
  if (typeof root.type === "function") return text(root.type(root.props));
  return [root.props.children]
    .flat(Infinity)
    .map((child) =>
      child && typeof child === "object" && "props" in child
        ? text(child)
        : typeof child === "string" || typeof child === "number"
          ? String(child)
          : "",
    )
    .join("");
}

const tabs = (tree: SegmentControlElement) =>
  nodes(tree).filter((node) => node.props.accessibilityRole === "tab");

test("grouped choices expose labels and counts and keep selection controlled by the parent", (t) => {
  const presses: number[] = [];
  const props: SegmentControlProps = {
    accessibilityLabel: "Plugins",
    segments: [
      {
        id: "added",
        label: "Added",
        accessibilityLabel: "Added connections",
        count: 0,
      },
      { label: "Explore", count: 3 },
    ],
    activeIndex: 0,
    onPress: (index) => presses.push(index),
  };
  const screen = setupSegmentControl(props);
  t.after(screen.harness.unmount);
  assert.equal(screen.tree().props.accessibilityRole, "tablist");
  assert.equal(screen.tree().props.accessibilityLabel, "Plugins");
  assert.deepEqual(
    tabs(screen.tree()).map((tab) => tab.props.accessibilityLabel),
    ["Added connections", "Explore"],
  );
  assert.deepEqual(tabs(screen.tree()).map(text), ["Added", "Explore3"]);
  tabs(screen.tree())[1]!.props.onPress();
  assert.deepEqual(presses, [1]);
  assert.deepEqual(
    tabs(screen.tree()).map((tab) => tab.props.accessibilityState.selected),
    [true, false],
  );
  screen.update({ ...props, activeIndex: 1 });
  assert.deepEqual(
    tabs(screen.tree()).map((tab) => tab.props.accessibilityState.selected),
    [false, true],
  );
  const addedKey = tabs(screen.tree())[0]!.key;
  screen.update({
    ...props,
    segments: [
      { ...props.segments[0]!, label: "Added · 0" },
      props.segments[1]!,
    ],
  });
  assert.equal(
    tabs(screen.tree())[0]!.key,
    addedKey,
    "refreshing a count preserves the tab's native identity",
  );
  assert.deepEqual(
    tabs(renderSegmentControl(props)).map(
      (tab) => tab.props.accessibilityState.selected,
    ),
    [true, false],
    "separate parent expansions keep independent hook state",
  );
});

test("disabled choices retain native and accessibility disabled semantics until the parent enables them", (t) => {
  const presses: number[] = [];
  const props: SegmentControlProps = {
    segments: [{ label: "Incoming" }, { label: "History" }],
    activeIndex: 1,
    disabled: true,
    onPress: (index) => presses.push(index),
  };
  const screen = setupSegmentControl(props);
  t.after(screen.harness.unmount);
  for (const tab of tabs(screen.tree())) {
    assert.equal(tab.props.disabled, true);
    assert.equal(tab.props.accessibilityState.disabled, true);
  }
  screen.update({ ...props, disabled: false });
  for (const tab of tabs(screen.tree())) {
    assert.equal(tab.props.disabled, false);
    assert.equal(tab.props.accessibilityState.disabled, false);
  }
  tabs(screen.tree())[0]!.props.onPress();
  assert.deepEqual(presses, [0]);
});

test("appearance reflows between horizontal and stacked choices while icons, scalable labels and selection survive", (t) => {
  const props: SegmentControlProps = {
    accessibilityLabel: "Appearance",
    segments: [
      {
        label: "Light",
        accessibilityLabel: "Light appearance",
        icon: createElement("Sun"),
      },
      {
        label: "Dark",
        accessibilityLabel: "Dark appearance",
        icon: createElement("Moon"),
      },
    ],
    activeIndex: 1,
    stacked: false,
    onPress: () => {},
  };
  const screen = setupSegmentControl(props);
  t.after(screen.harness.unmount);
  screen.layout(320);
  assert.ok(nodes(screen.tree()).some((node) => node.type === "Animated.View"));
  for (const fontScale of [2.143, 1]) {
    screen.scale(fontScale);
    screen.update({ ...props, stacked: fontScale > 1 });
    const tree = nodes(screen.tree());
    const containerStyle = Object.assign(
      {},
      ...[screen.tree().props.style].flat(Infinity).filter(Boolean),
    );
    assert.equal(
      containerStyle.flexDirection,
      fontScale > 1 ? "column" : "row",
    );
    assert.ok(tree.some((node) => node.type === "Sun"));
    assert.ok(tree.some((node) => node.type === "Moon"));
    assert.deepEqual(tabs(screen.tree()).map(text), ["Light", "Dark"]);
    assert.deepEqual(
      tabs(screen.tree()).map((tab) => tab.props.accessibilityState.selected),
      [false, true],
    );
    const labels = tree.filter((node) => node.type === "Text");
    assert.equal(labels.length, 2);
    for (const label of labels) {
      assert.equal(label.key, fontScale);
      assert.equal(label.props.numberOfLines, undefined);
      assert.notEqual(label.props.allowFontScaling, false);
    }
    assert.equal(
      tree.some((node) => node.type === "Animated.View"),
      fontScale === 1,
      "stacked controls do not retain a horizontal moving highlight",
    );
  }
});
