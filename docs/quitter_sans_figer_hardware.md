# HX Linux — Quitter l'application sans figer l'écran du hardware

> **Désabonnement explicite** (`sub=0x02` sur les 3 lanes), **synchrone** avant `exit(0)`, **espacé** ~150 ms entre chaque lane.
>
> **Confirmé sur capture** (`close_linux` final). En quittant, HXLinux coupait le poll et libérait l'interface **sans prévenir le hardware**, qui restait en mode éditeur (écran figé, moteur audio encore actif). Le correctif : envoyer, **avant `exit(0)`** et de façon **synchrone**, le tour de désabonnement observé sur HX Edit — un paquet `sub=0x02` sur chacune des 3 lanes (`f0`/`ed`/`ef`), **espacés de ~150 ms**. Le périphérique traite chaque désabonnement **à réception** ; l'ACK n'est pas requis. Écran rétabli sans reboot.

## 1. Le symptôme

Après fermeture de HXLinux : l'écran du Helix **ne se synchronise plus** (figé), mais les **commandes restent effectives** (footswitches, son). État **persistant** — reboot nécessaire. Signature d'un **mode éditeur jamais relâché** : le device croit qu'un éditeur le pilote encore et suspend sa gestion d'écran ; le moteur audio tourne indépendamment.

## 2. La cause racine

Le teardown ne faisait que `stop_all()` du poll puis `release_interface`. Aucun message « l'éditeur s'en va ». Hypothèse initiale « le drop USB suffit » → **infirmée** : ce n'est pas la libération de l'interface qui sort le hardware du mode éditeur, c'est un **désabonnement explicite** au niveau du protocole propriétaire.

## 3. La référence HX Edit (`08_close_HXEdit.json`)

Tout au long de la session, les keep-alives sont en `sub=0x10`. À l'instant du close, **un dernier tour** sur les 3 lanes passe en `sub=0x02` (le sous-type du *subscribe*), puis silence HID :

```
OUT 80 10 ed 03  sub=02      OUT 02 10 f0 03  sub=02      OUT 01 10 ef 03  sub=02
IN  ed 03 80 10  sub=02 ✓    IN  f0 03 02 10  sub=02 ✓    IN  ef 03 01 10  sub=02 ✓
— puis plus aucun trafic HID —
```

Chez HX Edit les 3 sont acquittés, mais (voir §5) ce n'est **pas** l'ACK qui compte : c'est la réception du `sub=0x02`, espacée.

Le paquet de close = le poll idle de la lane, **un seul octet changé** : `byte 11` passe de `0x10` à `0x02`. (C'est pour ça que l'étude initiale, faite à l'œil, n'avait rien trouvé.)

## 4. Le correctif — trois ingrédients

1. **Synchrone, avant `exit(0)`.** À la fermeture de l'application, le handler `CloseRequested` appelle `app.exit(0)` qui **tue le process immédiatement** : ni teardown différé, ni message via un canal, ne tourneraient à temps. Le tour `0x02` est émis **dans le handler**, en `write_bulk` direct, avant `exit(0)`.

2. **Espacé (~150 ms entre chaque lane).** C'est l'ingrédient décisif (§6). Balancés ensemble, le hardware n'en traite qu'un seul.

3. **Sans toucher `helix_session_stop`.** Ce drapeau déclenche `disconnect_helix_session` (`connected=false`, `tx=None`) dans un autre thread → le close se sabordait avant d'être émis. On stoppe **seulement** le poll idle via le canal keep-alive (`KeepAliveCommand::StopAll`), jamais la session.

## 5. Fausses pistes écartées (ne pas y revenir)

- **« Il faut que les 3 lanes soient ACK. »** Faux. Sur capture HXLinux fonctionnelle, **seul `ef` répond** ; `ed`/`f0` ne répondent **jamais** (ce sont des lanes *pilotées par le device* — un OUT host n'y est pas une requête à laquelle il répond). Et pourtant l'écran revient. Le device traite le désabonnement **à réception**, pas en répondant. L'ACK de `ef` n'est qu'un effet de bord (lane menée par le host).

- **« C'est un problème de compteur (collision / `device_last+1`). »** Faux, vérifié : après être passé à `device_last+1`, `ed` valait `40` (≠ `3f` du device, donc **sans collision**) et n'était toujours pas acquitté, tandis que le rythme seul a tout débloqué. Le compteur n'est pas le discriminant. *(Le code laisse `device_last+1` car inoffensif, mais ce n'était pas la cause.)*

## 6. Pourquoi c'est le rythme (preuve)

**Échec** (`close_linux` v1) — 3 closes en < 1 ms :
```
1.528 OUT f0 sub=02   1.528 OUT ed sub=02   1.528 OUT ef sub=02   1.529 IN ef sub=02 ✓
→ device n'en traite qu'un (le dernier) ; écran reste figé.
```

**Succès** (`close_linux` final) — closes espacés de ~150 ms :
```
1.800 OUT ed sub=02
1.951 OUT f0 sub=02      (+151 ms)
2.101 OUT ef sub=02      (+150 ms)   2.101 IN ef sub=02 ✓
→ device traite les 3 à réception ; écran rétabli.
```

Le délai vient du `read_bulk(0x81, …, 150 ms)` placé après chaque envoi : sur `ed`/`f0` (qui ne renvoient rien) son **timeout** fournit l'espacement ; sur `ef` il récupère l'ACK. **C'est le timeout qui est l'ingrédient actif** — d'où l'avertissement en §8.

## 7. L'implémentation

**`keep_alive.rs`** — source unique des paquets :

```rust
pub const CLOSE_SUB: u8 = 0x02;
pub fn graceful_close_packets(state: &mut HelixState) -> [Vec<u8>; 3] {
    // [f0, ed, ef] : poll idle de chaque lane, byte 11 = 0x02. (compteur device_last+1, non décisif)
}
```

**`lib.rs` — `graceful_helix_close(app)`**, appelé avant `exit(0)` :

1. stopper le poll idle (`keepalive_tx → StopAll`) + le listener ; **pas** `helix_session_stop` ; puis `sleep(80 ms)` pour laisser le poll et le listener libérer l'endpoint `0x81` ;
2. réordonner en `ed → f0 → ef` (ordre HX Edit) et, pour chaque lane : `write_bulk(0x01, close)` puis `read_bulk(0x81, …, 150 ms)` → l'envoi **et** l'espacement ;
3. `release_interface(0/4)` + `attach_kernel_driver`.

## 8. À retenir

- **Le mode éditeur est un abonnement protocolaire**, pas un effet de bord du lien USB : il faut le **fermer explicitement** (`sub=0x02` sur les 3 lanes).
- **Le rythme prime sur l'ACK.** Les closes doivent être **espacés** (~150 ms confirmé fonctionnel ; minimum réel du device non mesuré). Balancés ensemble → un seul traité.
- **Ne pas « optimiser » le `read_bulk`.** Sur `ed`/`f0` il ne reçoit rien, mais son timeout **est** l'espacement. Le retirer recasserait la fermeture.
- **`exit(0)` ne laisse aucun filet** : tout nettoyage matériel à la fermeture doit être **synchrone**, avant l'appel.
- **Ne jamais déclencher `disconnect_helix_session` depuis le chemin de close** (`helix_session_stop`) : il met `connected=false` / `tx=None` et saborde l'envoi. Couper uniquement le poll.

---

*Quitter proprement, c'est dire au revoir — lentement. Le hardware attend un désabonnement explicite sur chaque lane pour reprendre son écran, et il lui faut le temps de traiter chacun. Balancés d'un coup, il n'en entend qu'un ; espacés, il les entend tous. L'ACK, lui, est accessoire : c'est la parole reçue qui compte, pas la réponse.*
