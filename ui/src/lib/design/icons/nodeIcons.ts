// The icons a connection or folder can be given (Lucide, per-icon imports so only these 40 reach
// the bundle). The names and their order are the same as `crates/core-model/src/appearance.json`,
// which the Rust side validates against (see ../nodeAppearance.ts, which has a test for it).

import type { Component } from "svelte";

import { isNodeColor, isNodeIcon, type NodeIconName } from "../nodeAppearance";

import Server from "@lucide/svelte/icons/server";
import Database from "@lucide/svelte/icons/database";
import Monitor from "@lucide/svelte/icons/monitor";
import Terminal from "@lucide/svelte/icons/terminal";
import Globe from "@lucide/svelte/icons/globe";
import Cloud from "@lucide/svelte/icons/cloud";
import Folder from "@lucide/svelte/icons/folder";
import Building from "@lucide/svelte/icons/building";
import House from "@lucide/svelte/icons/house";
import Shield from "@lucide/svelte/icons/shield";
import Router from "@lucide/svelte/icons/router";
import Network from "@lucide/svelte/icons/network";
import HardDrive from "@lucide/svelte/icons/hard-drive";
import Cpu from "@lucide/svelte/icons/cpu";
import Box from "@lucide/svelte/icons/box";
import Container from "@lucide/svelte/icons/container";
import Lock from "@lucide/svelte/icons/lock";
import Key from "@lucide/svelte/icons/key";
import Star from "@lucide/svelte/icons/star";
import Flag from "@lucide/svelte/icons/flag";
import Briefcase from "@lucide/svelte/icons/briefcase";
import Wrench from "@lucide/svelte/icons/wrench";
import FlaskConical from "@lucide/svelte/icons/flask-conical";
import Laptop from "@lucide/svelte/icons/laptop";
import Smartphone from "@lucide/svelte/icons/smartphone";
import Printer from "@lucide/svelte/icons/printer";
import Camera from "@lucide/svelte/icons/camera";
import Radio from "@lucide/svelte/icons/radio";
import Zap from "@lucide/svelte/icons/zap";
import Activity from "@lucide/svelte/icons/activity";
import Code from "@lucide/svelte/icons/code";
import Bug from "@lucide/svelte/icons/bug";
import Cable from "@lucide/svelte/icons/cable";
import Wifi from "@lucide/svelte/icons/wifi";
import Mail from "@lucide/svelte/icons/mail";
import Users from "@lucide/svelte/icons/users";
import Layers from "@lucide/svelte/icons/layers";
import Package from "@lucide/svelte/icons/package";
import Rocket from "@lucide/svelte/icons/rocket";
import Heart from "@lucide/svelte/icons/heart";

export const NODE_ICONS: Record<NodeIconName, Component<any>> = {
  server: Server,
  database: Database,
  monitor: Monitor,
  terminal: Terminal,
  globe: Globe,
  cloud: Cloud,
  folder: Folder,
  building: Building,
  house: House,
  shield: Shield,
  router: Router,
  network: Network,
  "hard-drive": HardDrive,
  cpu: Cpu,
  box: Box,
  container: Container,
  lock: Lock,
  key: Key,
  star: Star,
  flag: Flag,
  briefcase: Briefcase,
  wrench: Wrench,
  "flask-conical": FlaskConical,
  laptop: Laptop,
  smartphone: Smartphone,
  printer: Printer,
  camera: Camera,
  radio: Radio,
  zap: Zap,
  activity: Activity,
  code: Code,
  bug: Bug,
  cable: Cable,
  wifi: Wifi,
  mail: Mail,
  users: Users,
  layers: Layers,
  package: Package,
  rocket: Rocket,
  heart: Heart,
};

/** The icon component for a stored icon name, or undefined when it is empty or not one of ours. */
export function nodeIcon(name: string | undefined): Component<any> | undefined {
  return isNodeIcon(name) ? NODE_ICONS[name] : undefined;
}

/** `var(--node-<colour>)` for a stored colour, or undefined when it is empty or not one of ours. */
export function nodeColorVar(color: string | undefined): string | undefined {
  return isNodeColor(color) ? `var(--node-${color})` : undefined;
}
