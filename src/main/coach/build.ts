import type { BuildPanelState, GameIntel, OpponentProfile } from '../../shared/intel';
import type { ItemData } from '../intel/ddragon';
import { callModel, type Effort } from './llm';

/**
 * AI build advisor: the next 2–3 items for me given the phase, my current items,
 * enemy damage types/items, who is fed, and the patch's meta build. Called on
 * real changes only (start, enemy completes an item, I die/recall, every ~3 min).
 */
const SCHEMA = {
  type: 'object', additionalProperties: false, required: ['phase', 'items'],
  properties: {
    phase: { type: 'string', maxLength: 20 },
    items: { type: 'array', minItems: 1, maxItems: 3, items: { type: 'object', additionalProperties: false, required: ['name'], properties: { name: { type: 'string' } } } },
  },
};

const SYSTEM = `Jesteś ekspertem od buildów League of Legends. Dobierz NASTĘPNE 2–3 przedmioty dla gracza (kolejność zakupu), uwzględniając:
- fazę gry, jego obecne przedmioty i złoto,
- typ obrażeń wrogów (AD/AP), ich przedmioty, leczenie, tarcze, CC, kto jest nafeedowany (najgroźniejszy),
- aktualny build z mety z briefu patcha (jeśli podany) — odchodź od niego tylko z konkretnego powodu.
Najpierw ustal, czym jest postać gracza ("me.class" z danych gry) i jak wygrywa walki, dopiero potem dobieraj przedmioty — nie licz samych statystyk. Burst (Mage + Assassin, np. Ahri, Syndra, LeBlanc, Zed): obrażenia na jedno combo i penetracja (Shadowflame, Rabadon's Deathcap, Stormsurge, Lich Bane, Void Staff gdy wróg ma odporność na magię) plus jeden przedmiot na przeżycie (Zhonya's albo Banshee's); Liandry's Torment i Rylai's to przedmioty dla magów zadających obrażenia długo (Brand, Cassiopeia, Vladimir, Swain) — burstowi daj je tylko przeciw 3+ wrogom, których GŁÓWNA klasa (pierwsza w "class") to Tank — Fighter z dopiskiem Tank (Urgot, Wukong, Darius) to nie tank. Przeciw zabójcom i nurkującym (Assassin, Fighter wchodzący w ciebie) — Zhonya's albo Plated Steelcaps; przeciw samym magom — Mercury's Treads / Banshee's Veil; przeciw tankom ("enemies[].class" Tank) — procentowa penetracja i obrażenia od maks. HP. Nazwy przedmiotów DOKŁADNIE z listy "availableItems" (angielskie nazwy z gry). Jeśli jest "finishFirst" (zaczęty przedmiot z planu — gracz ma już jego składniki), zwykle daj go jako pierwszy; inny tylko, gdy wyraźnie bardziej się opłaca (np. te składniki lepiej pasują do innego przedmiotu, który teraz bardziej się przyda). Gdy "me.boots" to "brak" albo zwykłe Boots, a minęło ~8 min, w planie MUSZĄ być buty tier 2 (np. Plated Steelcaps na AD i auto ataki, Mercury's Treads na AP i CC) jako pierwszy albo drugi przedmiot. Jeśli jest "currentPlan" (plan, który gracz już widzi), zostaw jego pierwszy przedmiot, chyba że jest konkretny nowy powód (przedmioty lub feed wrogów) — nie zmieniaj planu w tę i z powrotem. Jeśli w "me.componentsInto" gracz ma składnik, zwykle pierwszy przedmiot to jeden z tych, w które się składa (i pasuje do postaci) — nie zostawiaj kupionego składnika bez dokończenia. Bez uzasadnień — tylko nazwy.
"phase": "wczesna gra" / "środek gry" / "późna gra".`;

export function buildContext(intel: GameIntel, items: Map<number, ItemData>, profiles: OpponentProfile[], patchBrief: string | undefined, fed: string[]) {
  const me = intel.me!;
  const completed = [...items.values()].filter(i => i.completed && i.total >= 900);
  return {
    time: Math.round(intel.gameTime / 60) + ' min',
    me: { champion: me.champion, role: me.role, level: me.level, goldNow: intel.myStats?.gold, items: me.items.map(i => i.name), stats: intel.myStats },
    enemies: intel.players.filter(p => !p.isAlly).map(p => ({ champion: p.champion, role: p.role, kda: `${p.kills}/${p.deaths}/${p.assists}`, items: p.items.filter(i => i.price >= 900).map(i => i.name), fed: fed.includes(p.champion), profile: profiles.find(pr => pr.champion === p.champion)?.summary })),
    allies: intel.players.filter(p => p.isAlly && !p.isMe).map(p => p.champion),
    goldDiff: intel.goldDiff,
    patch: patchBrief,
    availableItems: completed.map(i => i.name),
  };
}

export async function adviseBuild(ctx: ReturnType<typeof buildContext>, items: Map<number, ItemData>, iconUrl: (id: number) => string, model = 'claude-sonnet-5-5', effort: Effort = 'low', tools?: string[]): Promise<BuildPanelState> {
  const r = await callModel<{ phase: string; items: Array<{ name: string }> }>({
    model, effort, tools, system: SYSTEM, schema: SCHEMA, prompt: JSON.stringify(ctx), timeoutMs: 120000,
  });
  const byName = new Map([...items.values()].map(i => [i.name.toLowerCase(), i]));
  return {
    phase: r.value.phase,
    updatedAt: Date.now(),
    items: r.value.items.slice(0, 3).map(it => {
      const data = byName.get(it.name.toLowerCase()) ?? [...items.values()].find(i => i.name.toLowerCase().includes(it.name.toLowerCase()));
      // Only the item: the user asked for no descriptions in the panel.
      return { name: data?.name ?? it.name, iconUrl: data ? iconUrl(data.id) : '', why: '', cost: data?.total ?? null };
    }),
  };
}

/** Decides when the build should be re-evaluated. */
export class BuildTrigger {
  private lastKey = '';
  private lastAt = 0;
  shouldRefresh(intel: GameIntel, now: number): boolean {
    const me = intel.me;
    if (!me) return false;
    const enemyItems = intel.players.filter(p => !p.isAlly).reduce((s, p) => s + p.completedItems, 0);
    const key = `${me.items.map(i => i.id).join(',')}|${enemyItems}|${me.isDead}`;
    if (key !== this.lastKey || now - this.lastAt > 180000) {
      this.lastKey = key; this.lastAt = now;
      return true;
    }
    return false;
  }
}
