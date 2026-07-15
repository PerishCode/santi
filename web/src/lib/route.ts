import type { ComponentType } from "react";
import { Window } from "../views/Window";

export const table: Record<string, ComponentType> = {
	"": Window,
	"#/": Window,
};

export function resolve(hash: string): ComponentType {
	return table[hash] ?? Window;
}
