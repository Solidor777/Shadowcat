import { PANEL_CONTRACT as e, getPointer as t } from "@shadowcat/core";
import "svelte/internal/disclose-version";
import * as n from "svelte/internal/client";
import { createSubscriber as r } from "svelte/reactivity";
import { getAppContext as i, setField as a } from "@shadowcat/ui-kit";
//#region src/InitiativePanel.svelte
var o = n.from_html("<li><span> </span> <button type=\"button\" class=\"svelte-9t6n0r\">Roll</button></li>"), s = n.from_html("<li> </li>"), c = n.from_html("<ol></ol> <p> </p> <button type=\"button\" class=\"svelte-9t6n0r\">Next turn</button>", 1), l = n.from_html("<div class=\"initiative svelte-9t6n0r\"><h3>Initiative</h3> <ul></ul> <!></div>");
function u(e, u) {
	n.push(u, !0);
	let p = i(), m = r((e) => p.documents.subscribe(e)), h = n.derived(() => (m(), p.documents.query("actor"))), g = n.state(n.proxy([])), _ = n.state(0), v = n.derived(() => n.get(g)[n.get(_)]);
	function y(e) {
		let r = d(() => Math.random());
		n.set(g, f([...n.get(g).filter((t) => t.actorId !== e.id), {
			actorId: e.id,
			name: e.name ?? "Unknown",
			initiative: r
		}]), !0), n.set(_, 0);
		let i = "/system/initiative";
		p.canEdit(e, i) && a(p, e.id, i, t(e, i), r);
	}
	function b() {
		n.get(g).length > 0 && n.set(_, (n.get(_) + 1) % n.get(g).length);
	}
	var x = l(), S = n.sibling(n.child(x), 2);
	n.each(S, 21, () => n.get(h), (e) => e.id, (e, t) => {
		var r = o(), i = n.child(r), a = n.child(i, !0);
		n.reset(i);
		var s = n.sibling(i, 2);
		n.reset(r), n.template_effect(() => n.set_text(a, n.get(t).name ?? "Unknown")), n.delegated("click", s, () => y(n.get(t))), n.append(e, r);
	}), n.reset(S);
	var C = n.sibling(S, 2), w = (e) => {
		var t = c(), r = n.first_child(t);
		n.each(r, 23, () => n.get(g), (e) => e.actorId, (e, t, r) => {
			var i = s();
			let a;
			var o = n.child(i);
			n.reset(i), n.template_effect(() => {
				a = n.set_class(i, 1, "svelte-9t6n0r", null, a, { active: n.get(r) === n.get(_) }), n.set_text(o, `${n.get(t).name ?? ""} — ${n.get(t).initiative ?? ""}`);
			}), n.append(e, i);
		}), n.reset(r);
		var i = n.sibling(r, 2), a = n.child(i);
		n.reset(i);
		var o = n.sibling(i, 2);
		n.template_effect(() => n.set_text(a, `Current: ${n.get(v)?.name ?? ""}`)), n.delegated("click", o, b), n.append(e, t);
	};
	n.if(C, (e) => {
		n.get(g).length > 0 && e(w);
	}), n.reset(x), n.append(e, x), n.pop();
}
n.delegate(["click"]);
//#endregion
//#region src/index.ts
function d(e) {
	return Math.floor(e() * 20) + 1;
}
function f(e) {
	return [...e].sort((e, t) => t.initiative - e.initiative || e.name.localeCompare(t.name));
}
var p = {
	manifest: {
		id: "example-initiative-tracker",
		version: "0.1.0",
		dependencies: {},
		requires: [e],
		provides: [],
		engines: { shadowcat: "^0.1.0" }
	},
	register(t) {
		t.contributions.contribute({
			id: "example-initiative-tracker:panel",
			contract: e,
			component: u,
			panel: {
				icon: "⚔️",
				labelKey: "Initiative",
				gmOnly: !0
			}
		});
	}
};
//#endregion
export { p as default, d as rollInitiative, f as sortEntries };
