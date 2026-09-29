import type { Messages } from "./en";

const fr: Messages = {
  "app.preAlpha": "pré-alpha",
  "app.language": "Langue",

  "toolbar.undoHint": "Annuler ({mod}+Z)",
  "toolbar.redoHint": "Rétablir ({mod}+Maj+Z)",

  "document.untitled": "Sans titre",

  "open.hint": "Ouvrir des images dans de nouveaux onglets ({mod}+O)",

  "tabs.newHint": "Nouveau document ({mod}+N)",
  "tabs.closeHint": "Fermer ({mod}+W, ou clic du milieu)",

  "welcome.title": "Ouvrez une image ou créez un document",
  "welcome.open": "Ouvrir…",
  "welcome.new": "Nouveau document",
  "welcome.drop": "…ou déposez des fichiers ici",

  "drop.newTab": "Déposez pour ouvrir dans un nouvel onglet",
  "drop.layer": "Déposez pour ajouter comme calque",
  "open.opening": "Ouverture de {name}…",
  "open.failed": "Impossible d'ouvrir {name} : {error}",
  "open.warning.iccProfileUnsupported":
    "Profil colorimétrique intégré pas encore pris en charge (à LUT) : couleurs lues en sRGB",
  "open.warning.iccCurveApproximated": "Courbe du profil colorimétrique approximée",
  "open.warning.firstFrameOnly": "Image animée : seule la première image a été ouverte",
  "open.warning.firstPageOnly": "Plusieurs pages : seule la première a été ouverte",
  "open.warning.precisionReduced": "Échantillons flottants 64 bits stockés en 32 bits",
  "open.warning.nonFiniteSamples":
    "Valeurs infinies ou indéfinies (NaN) : affichées comme la valeur la plus lumineuse ou 0",
  "open.warning.colorInfoUnsupported":
    "Informations de couleur du fichier pas encore prises en charge : couleurs lues en sRGB",

  "open.error.io": "lecture du fichier impossible ({detail})",
  "open.error.decode": "fichier endommagé ou image invalide ({detail})",
  "open.error.notYetSupported": "le format {detail} n'est pas encore pris en charge",
  "open.error.heic": "le HEIC/HEIF n'est pas pris en charge (brevets HEVC)",
  "open.error.unsupportedPixels": "ce type de pixels n'est pas encore pris en charge ({detail})",
  "open.error.tooLarge": "l'image est trop grande pour être ouverte ({detail})",
  "open.error.unrecognized": "format d'image non reconnu",
  "open.error.internal": "erreur interne ({detail})",
  "document.info": "{width} × {height} px · {space}",

  "colorSpace.srgb": "sRGB",
  "colorSpace.linear-srgb": "sRGB linéaire",
  "colorSpace.display-p3": "Display P3",
  "colorSpace.adobe-rgb": "Adobe RGB",
  "colorSpace.prophoto": "ProPhoto RGB",
  "colorSpace.rec2020": "Rec.2020",
  "colorSpace.linear-rec2020": "Rec.2020 linéaire",
  "colorSpace.rec2100-pq": "Rec.2100 PQ",
  "colorSpace.rec2100-hlg": "Rec.2100 HLG",
  "colorSpace.custom": "Personnalisé",

  "layers.title": "Calques",
  "layers.empty": "Aucun calque",
  "layers.show": "Afficher le calque",
  "layers.hide": "Masquer le calque",
  "layers.renameHint": "Double-cliquer ou F2 pour renommer",
  "layers.opacity": "Opacité",
  "layers.delete": "Supprimer le calque",
  "layers.fillColor": "Couleur de remplissage",
  "layers.addFill": "Ajouter un calque de remplissage",
  "layers.defaultFillName": "Remplissage {n}",

  "viewport.renderFailed": "Échec du rendu : {error}",

  "view.zoom": "Zoom",
  "view.hint":
    "Molette ou pincement : zoom · Bouton du milieu ou Espace+glisser : déplacer · {mod}+0 : ajuster · {mod}+1 : 100 % · {mod}+/- : paliers de zoom",

  "status.gpu": "GPU : {name} ({backend})",
  "status.gpuInit": "Initialisation du GPU…",
  "status.gpuUnavailable": "GPU indisponible : {error}",
  "status.frameTime": "rendu {render} ms · image {total} ms",
  "status.revision": "rév. {revision}",
};

export default fr;
