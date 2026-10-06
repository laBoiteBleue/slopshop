[English](README.md) · **Français**

# SlopShop

> Un éditeur d'images moderne et open source pour créer du slop de qualité professionnelle :
> non destructif, accéléré par le GPU, et nativement IA.

Le nom est une blague. L'ingénierie, non.

![SlopShop : une photo étalonnée par des calques de réglage dans un groupe, un filtre conservé comme entrée modifiable du calque, et l'éditeur de courbes](docs/images/screenshot-main.fr.jpg)

> [!NOTE]
> **SlopShop en est à ses débuts (pré-alpha).** Il permet déjà de vraies retouches : calques,
> sélections, peinture et retouche, réglages, filtres, transformations, et fichiers PSD avec
> leurs calques. Il n'a pas encore de texte, de formes ni d'IA générative, et a quelques
> défauts de jeunesse : gardez une copie des fichiers qui comptent pour vous, et
> [signalez ce qui ne marche pas](https://github.com/laBoiteBleue/slopshop/issues/new/choose)
> (en anglais ou en français).

## Télécharger

Les installeurs de chaque version sont sur la
[page des versions](https://github.com/laBoiteBleue/slopshop/releases).

| Système                                        | Installeur                  | Testé à la main |
| ---------------------------------------------- | --------------------------- | --------------- |
| Windows 10 et 11 (64 bits)                     | `.exe` (ou `.msi`)          | Oui             |
| macOS, Apple silicon et Intel                  | `.dmg`                      | **Non**         |
| Linux (x86-64) : Debian/Ubuntu, Fedora, autres | `.deb`, `.rpm`, `.AppImage` | **Non**         |

Le mainteneur teste uniquement sous Windows. Les versions macOS et Linux sont produites par
l'intégration continue, où les tests du moteur passent sur les deux systèmes, mais personne n'a
encore utilisé l'application sur ces systèmes : vos retours sont les bienvenus, même pour dire
que ça marche.

Les installeurs ne sont pas encore signés : Windows et macOS affichent un avertissement au
premier lancement. Les notes de version expliquent comment le passer sur chaque système. Il n'y
a pas encore de mise à jour automatique.

### Configuration minimale

|                               | Minimum                                                                                                                                                        |
| ----------------------------- | -------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| **Système**                   | Windows 10 ou 11 (64 bits) ; macOS 10.13 ou plus récent ; Linux x86-64 avec WebKitGTK 4.1 (Ubuntu 22.04, Debian 12, Fedora 38 ou plus récent)                  |
| **Carte graphique**           | Un GPU avec des pilotes DirectX 12 (Windows), Metal (macOS) ou Vulkan (Linux) ; SlopShop n'utilise que le niveau de base de ces API, puces intégrées comprises |
| **Mémoire**                   | 8 Go de RAM ; 16 Go ou plus pour des images de plusieurs centaines de mégapixels                                                                               |
| **Disque**                    | Environ 100 Mo, plus jusqu'à 1 Go environ pour les modèles d'IA facultatifs                                                                                    |
| **Outils d'IA (facultatifs)** | Windows x64 (DirectML, tout GPU DirectX 12), macOS sur Apple silicon (Core ML), Linux (sur le processeur). Indisponibles sur les Mac Intel.                    |

## Ce qu'il sait faire

- **Calques** : calques de pixels, groupes, masques d'écrêtage, masques de fusion, modes de
  fusion de Photoshop ; calques de réglage (Luminosité/Contraste, Niveaux, Courbes, Exposition,
  Vibrance, Teinte/Saturation, Balance des couleurs, Noir et blanc, Filtre photo, Mélangeur de
  couches, Négatif, Isohélie, Seuil, Courbe de transfert de dégradé, Correction sélective) ;
  calques de remplissage en couleur unie et en dégradé ; styles de calque (ombres, lueurs,
  contour, incrustation couleur) ; fusion et aplatissement, explicites et annulables.
- **Sélections** : rectangle et ellipse, lassos, baguette magique, sélection rapide, plage de
  couleurs ; sélection d’objet et Sélectionner le sujet par IA locale ; Sélectionner et masquer,
  avec affinage des contours ; masque rapide ; modifier, étendre, transformer, enregistrer et
  récupérer des sélections.
- **Peinture et retouche** : pinceau, crayon, gomme et gomme de restauration avec la pression
  du stylet, pot de peinture, dégradé, tampon de duplication, correcteur et pièce, densité −
  et +, goutte d'eau, netteté et doigt ; Édition > Remplir et Contour.
- **Filtres** : flou gaussien et flou directionnel, accentuation, passe-haut, ajout de bruit,
  anti-poussière, clarté et texture, fluidité.
- **Transformations** : déplacement avec magnétisme et repères commentés, alignement et
  répartition, transformation libre avec torsion et perspective, taille de l'image, taille
  de la zone de travail, rotation, recadrage avec redressement, rognage, règles et repères.
- **Fichiers** : ouvre plus de vingt formats dans leur précision d'origine (PNG, JPEG, TIFF,
  WebP, AVIF, JPEG XL, JPEG 2000, OpenEXR, Radiance HDR, DICOM, FITS, SVG, PDF, RAW
  d'appareils photo et d'autres), les PSD et PSB de Photoshop avec leurs calques ; exporte dans
  presque tous, les PSD et PSB avec leurs calques ; enregistre les documents dans le format
  propre de SlopShop, `.slop` (sans perte, incrémental, résistant aux plantages) ; imprime.
  Détails : [docs/formats.md](docs/formats.md) (en anglais).
- **Interface** : les menus, outils et raccourcis de Photoshop, des onglets de documents, les
  panneaux Historique, Histogramme et Infos, en français et en anglais.
- **Sans interface** : une commande `slopshop` fait des rendus et des conversions sans
  l'interface ([docs/cli.md](docs/cli.md), en anglais).

Pas encore : texte, formes, tracés et masques vectoriels ; IA générative (suppression,
remplissage génératif) ; modules et scripts ; enregistrement de l'historique d'annulation dans
les documents ; la vue native sur macOS. La [carte des fonctionnalités](docs/feature-map.md)
(en anglais) les recense toutes, faites ou non, et la [feuille de route](docs/roadmap.md) dit
ce qui vient ensuite.

## Pourquoi SlopShop

- **Non destructif de bout en bout.** Pas seulement les calques de réglage : les coups de
  pinceau, les filtres et les réglages appliqués à un calque sont des entrées de sa propre
  pile, chacune modifiable à nouveau, masquable et supprimable, au-dessus de pixels qui ne sont
  jamais réécrits. Les transformations repartent toujours de l'original, et un recadrage ne
  supprime rien.
- **Rapide, sur le GPU.** La composition, la plupart des filtres et les pointillés de sélection
  tournent sur le GPU (DirectX 12, Metal, Vulkan via [wgpu](https://wgpu.rs)), vérifiés par
  rapport à une référence sur le processeur. Les images sont découpées en tuiles, avec des
  niveaux de réduction, pour que les documents de centaines de mégapixels restent fluides.
- **Une couleur précise.** Entiers 8 et 16 bits, flottants 16 et 32 bits, HDR ; calques
  composés en lumière linéaire ; profils de couleur intégrés appliqués ; aucune conversion
  silencieuse ou avec perte.
- **Une IA locale et facultative.** Les outils d'IA tournent sur votre machine, après que vous
  avez accepté de télécharger chaque modèle et sa licence. Pas de compte, pas de cloud, aucun
  envoi. Ils ne sont jamais nécessaires pour utiliser l'éditeur.
- **Familier.** Les menus, outils et raccourcis de Photoshop, et les fichiers PSD avec leurs
  calques, en lecture comme en écriture.
- **Un logiciel libre, écrit en Rust.** Un moteur sûr en mémoire qui fonctionne sans
  l'interface, un format de document ouvert, et la liberté d'étudier, de modifier et de
  partager le tout.

### Où il va

Les modèles génératifs travaillent à 1 ou 2 mégapixels ; les images professionnelles en font
50 ou 100. Le pari à long terme de SlopShop est d'appliquer des outils génératifs à de telles
images **sans réduire leur résolution**, et sans toucher aux pixels hors de la zone modifiée.
L'approche n'est pas encore choisie et le sera par l'expérience : voir
[docs/research/hd-generative-ai.md](docs/research/hd-generative-ai.md) (en anglais).

## Captures d'écran

| Sélectionner le sujet par IA locale                                                                                             | Transformation libre en perspective                                                                                      |
| ------------------------------------------------------------------------------------------------------------------------------- | ------------------------------------------------------------------------------------------------------------------------ |
| ![Le sujet d'une photo sélectionné par le modèle d'IA local, entouré de pointillés](docs/images/screenshot-ai-selection.fr.jpg) | ![Un calque déformé avec les poignées de perspective de la transformation libre](docs/images/screenshot-perspective.jpg) |

## Compiler depuis les sources

Ce qu'il faut installer :

- [Rust](https://rustup.rs) (stable ; la version exacte est fixée par `rust-toolchain.toml`)
- [Node.js](https://nodejs.org) 24 ou plus, avec npm
- Les dépendances système de Tauri pour votre système : voir les
  [prérequis de Tauri](https://v2.tauri.app/start/prerequisites/) (WebView2 sous Windows, les
  Xcode Command Line Tools sous macOS, WebKitGTK et compagnie sous Linux)
- Sous Windows, le composant C++ ATL de Visual Studio en plus (Visual Studio Installer >
  Modifier > Composants individuels > « C++ ATL for latest build tools ») : le compilateur de
  shaders, DXC, est intégré et en a besoin ([ADR 0019](docs/adr/0019-dxc-shader-compiler.md))

Lancer l'application, ou construire ses installeurs :

```sh
cd app
npm install
npm run tauri dev     # version de développement
npm run bundle        # installeurs pour ce système, dans target/release/bundle/
```

Les commandes de vérification, le fonctionnement interne et l'architecture sont décrits dans
le [README anglais](README.md#building-from-source) et dans
[docs/architecture.md](docs/architecture.md).

## Contribuer

Les contributions sont les bienvenues, du signalement de bug au code : commencez par
[CONTRIBUTING.md](CONTRIBUTING.md) (en anglais, comme le code et les échanges sur le dépôt).
Tester sous macOS et Linux est l'aide la plus précieuse en ce moment. Les questions et les
idées vont dans les [Discussions](https://github.com/laBoiteBleue/slopshop/discussions) ; les
problèmes de sécurité suivent [SECURITY.md](SECURITY.md).

## Langues

Le code et la documentation sont en anglais. L'application est disponible en **français et en
anglais** (Édition > Préférences) ; ajouter une langue revient à ajouter un catalogue de
traduction (voir [ADR 0004](docs/adr/0004-ui-internationalization.md)).

## Licence

Copyright (C) 2026 The SlopShop contributors.

SlopShop est un logiciel libre, distribué sous la licence publique générale GNU, version 3
uniquement (`GPL-3.0-only`). Vous pouvez l'utiliser, l'étudier, le modifier et le redistribuer
selon les termes de cette licence ; les conditions exactes sont dans [LICENSE](LICENSE). Seul
le texte anglais du [README](README.md#license) et de la licence fait foi.

La spécification du format `.slop` ([docs/file-format.md](docs/file-format.md)) est sous
[CC BY 4.0](LICENSES/CC-BY-4.0.txt) et ses fichiers de référence sous
[CC0 1.0](LICENSES/CC0-1.0.txt), pour que d'autres logiciels puissent implémenter le format.

Les dépendances tierces gardent leurs propres licences. Les modèles et moteurs d'IA que
SlopShop peut télécharger à la demande ne font pas partie de SlopShop : chacun a sa propre
licence, affichée avant le téléchargement.

Le nom « SlopShop » et le logo du projet ne sont pas sous licence GPL : les droits éventuels
sur le nom et l'identité visuelle sont distincts de la licence du code.

Les photos des captures d'écran sont dans le domaine public (CC0), issues de Wikimedia
Commons :
[San Juan Valley](https://commons.wikimedia.org/wiki/File:San_Juan_Valley.jpg) par Wilfredor,
[Lotus flower](<https://commons.wikimedia.org/wiki/File:Lotus_flower_(978659).jpg>) par Hong
Zhang, et
[Scuol-Motta Naluns](<https://commons.wikimedia.org/wiki/File:Scuol-Motta_Naluns,_15-09-2023._(actm.)_09.jpg>)
par Agnes Monkelbaan.

SlopShop est un projet indépendant, sans lien avec Adobe, ni approuvé ni soutenu par Adobe.
Adobe et Photoshop sont des marques déposées ou des marques commerciales d'Adobe aux
États-Unis et/ou dans d'autres pays.
