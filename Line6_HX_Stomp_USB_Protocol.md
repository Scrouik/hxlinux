## Structure de la trame "Write Parameter"

Les commandes host → device transitent sur :
- **Endpoint** : `0x01` (OUT)
- **Transfer type** : `0x03` (Interrupt)  
- **Interface class** : `0xff` (Vendor Specific)
- **Opcode** : `80:10:ed:03`

**Note** : une autre famille de messages (`03:10:ed:03` sur le même préfixe `27:00:00:18`) apparaît dans d’autres contextes (ex. essais côté Linux / autres couches) ; **les captures HX Edit dans `src/Paquets Json/` (avril 2026) n’utilisent que `80:10:ed:03`** pour les writes paramètre 48 octets analysés ici.

### Trame complète (48 bytes)

```
Offset  Bytes                                    Description
------  -----                                    -----------
 0- 3   27:00:00:18                              Header (len=0x27=39, cmd=0x18)
 4- 7   80:10:ed:03                              Opcode "write parameter" (fixe)
 8-11   00:SEQ:00:04                             SEQ = numéro de séquence message
12-15   CTR_L:CTR_H:00:00                        Compteur de transaction 16-bit LE
16-23   01:00:06:00:17:00:00:00                  Constante (fixe)
24-27   83:66:cd:PP                              Identifiant paramètre (voir section PP)
28-31   YY:64:1e:65                              YY = compteur / index local (voir section YY)
32-42   85:62:01:1d:c3:1a:00:1c:00:77:ca        Constante (fixe)
43-46   F0:F1:F2:F3                              Valeur float IEEE 754 Big-Endian
47      00                                       Terminateur
```

---

## Champs variables

### Numéro de séquence `SEQ` (offset 9)
Compteur 8-bit qui progresse avec le trafic HX Edit (pas forcément +1 strict à chaque trame utile, car d’autres messages utilisent aussi la séquence). Pour une injection externe, l’hypothète raisonnable est de **reprendre la continuité observée** ou d’incrémenter proprement sans collision avec le reste de la session.

### Compteur de transaction `CTR_L:CTR_H` (offsets 12-13)
Compteur 16-bit little-endian. **Correction (vérifiée sur les exports JSON `src/Paquets Json/`)** : entre deux trames `80:10:ed:03` consécutives du même flux d’édition, le pas observé est **`+0x1F` (31 décimal)**, pas `+0x19` (25). Exemple (Heir Apparent, Gain) :

```
… ctr 3b:87 → 5a:87 → 79:87 → 98:87 → b7:87 → d6:87 …
```

Chaque pas : `+0x1F`. Il reste possible que d’autres firmwares / sessions utilisent un autre pas ; la valeur sûre pour rejouer **ces** captures est **`0x1F`**.

### Identifiant paramètre `PP` (offset 27)
`83:66:cd` est un **préfixe fixe** dans les captures analysées.  
**Correction importante** : l’hypothèse « `PP` = index 1-based du paramètre dans le JSON HX, avec `PP = 0x03 + (n−1)` » **ne colle pas** aux fichiers JSON fournis (même preset, slot 1). Il faut traiter `PP` comme un **identifiant à caler par capture / par modèle**, pas comme une règle universelle dérivée du seul ordre `params[]` du JSON.

#### Table observée (exports Wireshark, avril 2026)

Même contexte annoncé : preset **Preset Test**, **slot 1**. Fichiers dans `src/Paquets Json/`.

| Fichier capture | Paramètre UI | `PP` (octet 27) observé | Floats normalisés (OK) |
|-----------------|--------------|-------------------------|-------------------------|
| Minotaur Tone … | Tone | `03` puis `04` sur la dernière valeur | 0.53, 0.72, 0.27 |
| Minotaur Level … | Level | `04` partout | 0.60, 0.81, 0.29 |
| Heir Apparente Gain … | Gain | `04` partout | 0.50, 0.72, 0.29 |
| Heir Apparente Tone … | Tone | `04` partout | 0.50, 0.69, 0.31 |

**À retenir** : pour **Minotaur Tone**, HX Edit envoie d’abord plusieurs trames avec **`PP=03`**, puis des trames avec **`PP=04`** pour la valeur finale — ce n’est pas un seul index stable « Tone = toujours `04` » sur toute la séquence. Cela suggère **plusieurs writes par geste** (paire / rafraîchissement / autre couche), pas un mapping trivial 1 paramètre = 1 `PP` constant.

L’idée qualitative reste valable : **le client (HX Edit) sait quel bloc / quel réglage est actif** ; le détail exact de `PP` reste à **cartographier par modèle et par type de paramètre** à partir de captures propres.

### Champ `YY` (offset 28)
**Correction** : dans les mêmes JSON, **`YY` s’incrémente** (souvent +1 entre writes successifs du même fichier). Ce n’est donc **pas** un champ qu’on peut raisonnablement figer à `0x00` si l’objectif est de mimer HX Edit. Pour un outil tiers, stratégies possibles : reprendre la dernière valeur vue +1, ou rejouer une capture octet pour octet hors float.

### Paires de trames par « pas » utilisateur
Sur chaque fichier test (trois valeurs affichées), on compte **six** trames OUT `80:10:ed:03` de 48 octets. Hypothèse plausible : **deux messages par changement** de valeur (ex. début/fin de drag, ou write + echo de contrôle côté client). À documenter plus finement avec corrélation frame number / timing.

### Valeur float `F0:F1:F2:F3` (offsets 43-46)
Float IEEE 754 **Big-Endian**, plage `0.0` à `1.0` normalisée sur les contrôles testés.

La valeur affichée sur le Stomp (ex: `4.2` sur une plage `0-10`) se convertit ainsi :
```
float = valeur_affichée / plage_max
```
Exemples cohérents avec les captures :
```
Gain 4.2  → 0.42 → bytes: 3e:d7:0a:3d
Gain 6.8  → 0.68 → bytes: 3f:2e:14:7b
Tone 5.3  → 0.53 → bytes: 3f:07:ae:14
Level 8.1 → 0.81 → bytes: 3f:2e:14:7b  (même motif hex qu’un autre float proche ; vérifier le contexte)
```

Conversion en Python :
```python
import struct
def encode_float(value: float) -> bytes:
    return struct.pack('>f', value)
```

---

## Construction d'une trame complète

Exemple : trame **calquée sur une capture** (opcode `80:10:ed:03`), avec **`PP`** et **`YY`** passés explicitement (recommandé tant que la règle `PP` n’est pas figée).

```python
import struct

def build_write_param(pp: int, yy: int, value: float, seq: int, ctr: int) -> bytes:
    """
    pp, yy    : octets issus d'une capture du même modèle / même session quand possible
    value     : float normalisé 0.0 à 1.0
    seq       : octet séquence (offset 9)
    ctr       : compteur 16-bit transaction ; entre deux sends testés sur nos JSON : +0x1F
    """
    ctr_l = ctr & 0xFF
    ctr_h = (ctr >> 8) & 0xFF
    float_bytes = struct.pack('>f', value)  # Big-Endian

    frame = bytes([
        0x27, 0x00, 0x00, 0x18,             # header
        0x80, 0x10, 0xed, 0x03,             # opcode
        0x00, seq & 0xFF, 0x00, 0x04,       # séquence
        ctr_l, ctr_h, 0x00, 0x00,           # compteur transaction
        0x01, 0x00, 0x06, 0x00,             # constante
        0x17, 0x00, 0x00, 0x00,             # constante
        0x83, 0x66, 0xcd, pp & 0xFF,        # param_id
        yy & 0xFF, 0x64, 0x1e, 0x65,        # YY + constante
        0x85, 0x62, 0x01, 0x1d,             # constante
        0xc3, 0x1a, 0x00, 0x1c,             # constante
        0x00, 0x77, 0xca,                   # constante
    ]) + float_bytes + bytes([0x00])        # float + terminateur

    return frame

# Exemple chiffré (valeurs fictives de pp/yy/seq/ctr — à remplacer par une vraie capture)
frame = build_write_param(pp=0x04, yy=0x24, value=0.65, seq=0x52, ctr=0x873B)
print(frame.hex(':'))
```

---

## Envoi via libusb

```python
import usb.core
import usb.util

VENDOR_ID  = 0x0E41
PRODUCT_ID = 0x4253
ENDPOINT_OUT = 0x01

dev = usb.core.find(idVendor=VENDOR_ID, idProduct=PRODUCT_ID)
if dev is None:
    raise ValueError("HX Stomp XL non trouvé")

dev.set_configuration()

seq = 0x00
ctr = 0x0000

frame = build_write_param(pp=0x04, yy=0x00, value=0.65, seq=seq, ctr=ctr)
dev.write(ENDPOINT_OUT, frame)

seq = (seq + 1) & 0xFF
ctr = (ctr + 0x1F) & 0xFFFF   # pas observé sur les JSON avril 2026 (pas 0x19)
```

---

## Changement de slot actif : device → host (HX Edit met à jour l’UI)

Source : export Wireshark **`src/Paquets Json/Slot1 to slot2 hardware.json`** (avril 2026) — l’utilisateur change le **slot sélectionné sur le hardware** (Stomp) pendant qu’**HX Edit** est connecté ; l’UI suit. Ce qui suit décrit **uniquement** ce que le **boîtier envoie vers le PC** sur l’endpoint **IN `0x81`** ; l’ordre **PC → hardware** pour imposer le même slot depuis l’UI reste à capturer séparément.

### Canal

| Élément | Valeur |
|--------|--------|
| Direction | **Device → host** (`usb.src` = adresse appareil, `usb.dst` = host) |
| Endpoint | **`0x81`** (IN, numéro 1) |
| Taille utile | **16 octets** (`usb.capdata` = 32 caractères hex + `:`) |

### Séquence observée (premier geste « slot 1 → slot 2 » dans le fichier)

Les deux messages arrivent **dans cet ordre** sur `0x81` (numéros de trame Wireshark indicatifs) :

1. **Trame courte « ED03 »** (`frame.number` **139**)  
   `usb.capdata` tel qu’exporté :  
   `08:00:00:18:ed:03:80:10:00:b8:00:10:c1:02:00:00`  

   - Préfixe commun : `08:00:00:18`  
   - Octets 4–7 : `ed:03:80:10` — même famille que l’opcode **`80:10:ed:03`** des trames OUT longues / keep-alive x80, avec **ordre des octets inversé** par rapport aux paquets **host → device** documentés plus haut (`80:10:ed:03`).  
   - Octets 8–15 : `00:b8:00:10:c1:02:00:00` — `b8` est un compteur / séquence sur le flux ; les octets **`c1:02`** sont **identiques** sur toutes les occurrences de ce type dans **ce** JSON (plusieurs transitions) ; le rôle exact de `c1` / `02` (slot, type d’événement, etc.) reste **à confirmer** avec d’autres captures (autres slots, autres presets).

2. **Trame courte « EF03 »** (`frame.number` **205**)  
   `usb.capdata` :  
   `08:00:00:18:ef:03:01:10:00:bb:00:10:65:02:00:00`  

   - Même préfixe `08:00:00:18`  
   - Octets 4–7 : `ef:03:01:10` — aligné sur la famille **`01:10:ef:03`** déjà vue côté host (keep-alive x1, `request_preset_names`, etc.), ici en provenance du **device**.  
   - Octets 8–15 : `00:bb:00:10:65:02:00:00` — **`65:02`** est constant sur les occurrences EF IN de ce fichier.

### Réaction HX Edit (host → device), pour contexte seulement

Quelques dizaines de millisecondes après cette paire IN, **HX Edit** envoie sur **OUT `0x01`** une **paire** de 16 octets documentée ailleurs dans le dépôt (réponse logicielle, pas émise par le Stomp) : d’abord `08:…:80:10:ed:03:…:00:10:…` puis `08:…:01:10:ef:03:…:00:10:ab:24:00:00`. Ce n’est **pas** la chaîne « hardware → UI » ; c’est la réaction du client Windows. **HXLinux** n’a pas à la rejouer tant que l’objectif est seulement de **décoder** la notification device.

### Implémentation future (HXLinux)

- Parser le **flux IN `0x81`** : détecter la paire **16 + 16** octets (schémas ci-dessus), puis mettre à jour l’index de slot sélectionné côté UI si les octets de queue se confirment slot-dépendants sur nouvelles captures.  
- Ne pas confondre avec le keep-alive **40 octets** `1d:00:00:18:f0:03:…` qui continue en parallèle sur le même endpoint.

### Capture **non disponible** à ce jour : HX Edit « lit » un preset et cible le slot actif

Quand **HX Edit** recharge / synchronise un preset avec le Stomp, l’interface se **place sur le slot actuellement actif** sur le boîtier. La chaîne USB exacte (ordre des paquets, champs portant l’index de slot, distinction avec un simple `RequestPreset` / dump déjà connu côté HXLinux) **n’a pas encore** été enregistrée dans `src/Paquets Json/`.

**Conseil pour l’export** : scénario minimal — HX Edit connecté, **un seul** enchaînement « ouverture preset / resync » (ex. double-clic sur un autre preset puis retour, ou fermer-réouvrir l’éditeur de preset) pendant que le Stomp reste sur un **slot non nul** (ex. slot 5 actif), fenêtre **2–5 s**, export JSON avec trafic **OUT `0x01`** et **IN `0x81`** (pas seulement HID `0x84`). Comparer ensuite avec la section *Changement de slot* ci-dessus pour isoler ce qui est **spécifique** à la « prise de focus slot » après lecture preset.

---

## Ce qui reste à explorer

- **Lecture de paramètre** : les trames device → host (endpoint `0x81`) contiennent probablement l'état courant des paramètres — structure non encore décodée.
- **Changement de preset** : opcode différent, non capturé.
- **HX Edit : lecture / resync preset → UI sur le slot actif** : capture USB **manquante** ; probablement distincte de la seule paire IN « changement de slot au pavé » (`Slot1 to slot2 hardware.json`) et du flux `request_preset_content` HXLinux.
- **Identification du slot** : writes param listés sur slot 1 ; slot 2+ et **octets `c1:02` / `65:02` de la paire IN slot** — corrélation à valider sur exports dédiés ; **ordre UI → hardware** (clic slot dans HX Edit) encore non capturé.
- **Règle exacte de `PP`** : corrélation avec l’ordre DSP, le catalogue HX, ou un index interne bloc — à stabiliser avec d’autres modèles et slots.
- **Pourquoi deux trames par pas** (6 writes pour 3 réglages) : corrélation avec UI / ACK.

---

## Références

- Captures réalisées avec Wireshark 4.x + USBPcap sous Windows 11
- Device : Line 6 HX Stomp XL (`VID_0E41 / PID_4253`)
- Exports JSON analysés automatiquement : `src/Paquets Json/*.json`
- JSON modèles HX : ordre `params[]` utile pour l’UI, mais **non prouvé** comme définition unique de `PP` sur le bus
- Projet Tauri d'origine : contrôle temps réel du Stomp XL depuis une UI desktop Linux
