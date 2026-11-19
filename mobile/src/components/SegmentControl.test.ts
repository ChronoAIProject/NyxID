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
    const labels = tabs(screen.tree()).flatMap((tab) =>
      nodes(tab).filter((node) => node.type === "Text"),
    );
    assert.equal(labels.length, 2);
    for (const label of labels) {
      assert.equal(label.key, fontScale);
      assert.equal(label.props.numberOfLines,
        undefined,
        "fitting and stacked labels retain base wrapping",);
      assert.notEqual(label.props.allowFontScaling, false);
    }
    assert.equal(
      tree.some((node) => node.type === "Animated.View"),
      fontScale === 1,
      "stacked controls do not retain a horizontal moving highlight",
    );
  }
});

const filters: SegmentControlProps = {
  accessibilityLabel: "Filter notifications",
  segments: [
    { id: "all", label: "All", count: 1 },
    { id: "decision", label: "Needs you" },
    { id: "automations", label: "Automations", count: 1 },
    { id: "chats", label: "Chats" },
  ],
  activeIndex: 2,
  onPress: () => {},
};
const style = (node: SegmentControlElement) =>
  Object.assign(
    {},
    ...[
      typeof node.props.style === "function"
        ? node.props.style({ pressed: false })
        : node.props.style,
    ]
      .flat(Infinity)
      .filter(Boolean),
  );

test("segment labels stay on one line at the Dynamic Type size; a row that cannot fit scrolls", (t) => {
  const screen = setupSegmentControl(filters);
  t.after(screen.harness.unmount);
  // 383pt page minus margins: a 320pt track (316pt inside its 2pt padding).
  screen.layout(320);
  const scrollView = () =>
    nodes(screen.tree()).find((node) => node.type === "ScrollView")!;
  const highlight = () =>
    nodes(screen.tree()).find((node) => node.type === "Animated.View")!;
  const widths = () => tabs(screen.tree()).map((tab) => style(tab).width);
  screen.measure([70, 140, 190, 90]);
  for (const tab of tabs(screen.tree())) {
    const label = nodes(tab).find((node) => node.type === "Text")!;
    assert.equal(label.props.numberOfLines, 1);
    assert.equal(label.props.adjustsFontSizeToFit, undefined, "never scaled down",);
    assert.equal(label.props.minimumFontScale, undefined);
  }
  // Every label fits an equal share: today's equal segments, no scrolling.
  screen.measure([50, 75, 60, 60]);
  assert.deepEqual(
    tabs(screen.tree()).map((tab) => style(tab).flex), [1, 1, 1, 1],);
  assert.equal(scrollView(), undefined);
  // "Automations" needs more than a share but the row still fits: natural
  // widths plus an equal share of the spare space, still no scrolling.
  screen.measure([50, 80, 110, 60]);
  assert.deepEqual(widths(), [52.5, 82.5, 112.5, 62.5]);
  assert.equal(scrollView().props.scrollEnabled, false);
  // Too wide for the track (largest text): natural widths, and it scrolls.
  const scrolls: { x: number }[] = [];
  scrollView().props.ref.current = {
    scrollTo: (to: { x: number }) => scrolls.push(to),
  };
  screen.measure([70, 140, 190, 90]);
  assert.deepEqual(widths(), [70, 140, 190, 90]);
  assert.equal(scrollView().props.scrollEnabled, true);
  assert.equal(scrollView().props.horizontal, true);
  // The highlight follows the selected segment's own width and offset, and
  // the row brings it into view.
  assert.equal(style(highlight()).width, 190);
  assert.equal(
    style(highlight()).transform[0].translateX.value, 70 + 2 + 140 + 2,);
  assert.ok(scrolls.length > 0);
  assert.ok(scrolls.at(-1)!.x > 0);
  screen.update({ ...filters, activeIndex: 0 });
  assert.equal(style(highlight()).width, 70);
  assert.equal(scrolls.at(-1)!.x, 0);
  // A count badge grows with its scaled number instead of clipping it.
  const badge = nodes(tabs(screen.tree())[0]!).find(
    (node) => node.type === "View" && style(node).minWidth === 18,
  )!;
  assert.equal(style(badge).height, undefined);
  assert.equal(style(badge).minHeight, 18);
  // Measuring copies are invisible to touch and accessibility.
  const clip = nodes(screen.tree()).find(
    (node) => node.props.accessibilityElementsHidden === true,
  )!;
  assert.equal(clip.props.pointerEvents, "none");
  assert.equal(clip.props.importantForAccessibility, "no-hide-descendants");
  assert.equal(
    nodes(clip).some((node) => node.props.accessibilityRole === "tab"),
    false,
  );
});

test("P9 fitting labels use the base equal cells and fixed badge without a scroll container", (t) => {
  const x = setupSegmentControl(filters);
  t.after(x.harness.unmount);
  x.layout(320);
  x.measure([50, 70, 70, 65]);
  assert.equal(
    nodes(x.tree()).some((n) => n.type === "ScrollView"),
    false,
  );
  for (const tab of tabs(x.tree())) {
    assert.equal(
      style({
        ...tab,
        props: { ...tab.props, style: tab.props.style({ pressed: false }) },
      }).flex,
      1,
    );
  }
  const badge = nodes(tabs(x.tree())[0]!).find(
    (n) => style(n).minWidth === 18,
  )!;
  assert.equal(style(badge).height, 18);
  assert.equal(style(badge).minHeight, undefined);
  const highlight = nodes(x.tree()).find((n) => n.type === "Animated.View")!;
  assert.equal(style(highlight).top, style(x.tree()).padding);
  assert.equal(style(highlight).left, style(x.tree()).padding);
});
