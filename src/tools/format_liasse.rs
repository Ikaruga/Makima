//! Outil de formatage des liasses fiscales extraites en texte brut
//!
//! Reformate le texte extrait d'une liasse fiscale PDF pour :
//! - Séparer clairement les formulaires (2050, 2051, 2052, etc.)
//! - Structurer les codes et libellés
//! - Vérifier la présence des formulaires attendus

use super::registry::{Tool, ToolResult};
use crate::llm::types::ParsedToolCall;
use anyhow::Result;
use async_trait::async_trait;
use regex::Regex;
use serde_json::json;
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use tokio::fs;

// =============================================================================
// DICTIONNAIRES CERFA - Codes comptables standards
// =============================================================================

/// Codes du formulaire 2050 (BILAN - ACTIF)
const CODES_2050: &[(&str, &str)] = &[
    ("AA", "Capital souscrit non appelé"),
    ("AB", "Frais d'établissement"),
    ("CX", "Frais de développement"),
    ("AF", "Concessions, brevets et droits similaires"),
    ("AH", "Fonds commercial"),
    ("AJ", "Autres immobilisations incorporelles"),
    ("AL", "Avances et acomptes sur immobilisations incorporelles"),
    ("AN", "Terrains"),
    ("AP", "Constructions"),
    ("AR", "Installations techniques, matériel et outillage industriels"),
    ("AT", "Autres immobilisations corporelles"),
    ("AV", "Immobilisations en cours"),
    ("CS", "Avances et acomptes"),
    ("CU", "Participations évaluées mise en équivalence"),
    ("BB", "Autres participations"),
    ("BD", "Créances rattachées à des participations"),
    ("BF", "Autres titres immobilisés"),
    ("BH", "Prêts"),
    ("BJ", "Autres immobilisations financières"),
    ("BL", "TOTAL (II) - Actif immobilisé"),
    ("BN", "Matières premières, approvisionnements"),
    ("BP", "En cours de production de biens"),
    ("BR", "En cours de production de services"),
    ("BT", "Produits intermédiaires et finis"),
    ("BV", "Marchandises"),
    ("BX", "Avances et acomptes versés sur commandes"),
    ("BZ", "Clients et comptes rattachés"),
    ("CB", "Autres créances"),
    ("CD", "Capital souscrit et appelé, non versé"),
    ("CF", "Valeurs mobilières de placement"),
    ("CH", "Disponibilités"),
    ("CJ", "Charges constatées d'avance"),
    ("CW", "TOTAL (III) - Actif circulant"),
    ("CM", "Charges à répartir sur plusieurs exercices"),
    ("CN", "Primes de remboursement des obligations"),
    ("CO", "Écarts de conversion actif"),
    ("1A", "TOTAL GÉNÉRAL (I à VI)"),
];

/// Codes du formulaire 2050 - colonnes Amortissements et Net
const CODES_2050_AMORT: &[(&str, &str)] = &[
    ("AC", "Capital souscrit non appelé - Amort."),
    ("CQ", "Frais d'établissement - Amort."),
    ("AI", "Frais de développement - Amort."),
    ("AK", "Concessions, brevets - Amort."),
    ("AM", "Fonds commercial - Amort."),
    ("AO", "Autres immob. incorporelles - Amort."),
    ("AS", "Terrains - Amort."),
    ("AU", "Constructions - Amort."),
    ("AW", "Installations techniques - Amort."),
    ("AY", "Autres immob. corporelles - Amort."),
    ("CT", "Immobilisations en cours - Amort."),
    ("CV", "Avances et acomptes - Amort."),
    ("BC", "Participations - Amort."),
    ("BE", "Créances rattachées - Amort."),
    ("BG", "Autres titres immobilisés - Amort."),
    ("BI", "Prêts - Amort."),
    ("BK", "Autres immob. financières - Amort."),
    ("BM", "TOTAL (II) - Amort."),
    ("BO", "Matières premières - Amort."),
    ("BQ", "En cours production biens - Amort."),
    ("BS", "En cours production services - Amort."),
    ("BU", "Produits finis - Amort."),
    ("BW", "Marchandises - Amort."),
    ("BY", "Avances versées - Amort."),
    ("CA", "Clients - Amort."),
    ("CC", "Autres créances - Amort."),
    ("CE", "Capital appelé non versé - Amort."),
    ("CG", "VMP - Amort."),
    ("CI", "Disponibilités - Amort."),
    ("CK", "Charges constatées d'avance - Amort."),
];

/// Codes du formulaire 2051 (BILAN - PASSIF)
const CODES_2051: &[(&str, &str)] = &[
    ("DA", "Capital social ou individuel"),
    ("DB", "Primes d'émission, de fusion, d'apport"),
    ("DC", "Écarts de réévaluation"),
    ("DD", "Écart d'équivalence"),
    ("DE", "Réserve légale"),
    ("DF", "Réserves statutaires ou contractuelles"),
    ("DG", "Réserves réglementées"),
    ("DH", "Autres réserves"),
    ("DI", "Report à nouveau"),
    ("DJ", "Résultat de l'exercice"),
    ("DK", "Subventions d'investissement"),
    ("DL", "Provisions réglementées"),
    ("DM", "TOTAL (I) - Capitaux propres"),
    ("DN", "Produit des émissions de titres participatifs"),
    ("DO", "Avances conditionnées"),
    ("DP", "TOTAL (II) - Autres fonds propres"),
    ("DQ", "Provisions pour risques"),
    ("DR", "Provisions pour charges"),
    ("DS", "TOTAL (III) - Provisions"),
    ("DT", "Emprunts obligataires convertibles"),
    ("DU", "Autres emprunts obligataires"),
    ("DV", "Emprunts et dettes auprès des établissements de crédit"),
    ("DW", "Emprunts et dettes financières divers"),
    ("DX", "Avances et acomptes reçus sur commandes en cours"),
    ("DY", "Dettes fournisseurs et comptes rattachés"),
    ("DZ", "Dettes fiscales et sociales"),
    ("EA", "Dettes sur immobilisations et comptes rattachés"),
    ("EB", "Autres dettes"),
    ("EC", "Produits constatés d'avance"),
    ("ED", "TOTAL (IV) - Dettes"),
    ("EE", "Écarts de conversion passif"),
    ("EF", "TOTAL GÉNÉRAL (I à V)"),
];

/// Codes du formulaire 2052 (COMPTE DE RÉSULTAT - I)
const CODES_2052: &[(&str, &str)] = &[
    ("FA", "Ventes de marchandises"),
    ("FB", "Production vendue biens"),
    ("FC", "Production vendue services"),
    ("FD", "Chiffre d'affaires net"),
    ("FE", "Production stockée"),
    ("FF", "Production immobilisée"),
    ("FG", "Subventions d'exploitation"),
    ("FH", "Reprises sur amort. et provisions, transferts de charges"),
    ("FI", "Autres produits"),
    ("FJ", "TOTAL DES PRODUITS D'EXPLOITATION (I)"),
    ("FK", "Achats de marchandises"),
    ("FL", "Variation de stock marchandises"),
    ("FM", "Achats de matières premières et autres approvisionnements"),
    ("FN", "Variation de stock matières premières"),
    ("FO", "Autres achats et charges externes"),
    ("FP", "Impôts, taxes et versements assimilés"),
    ("FQ", "Salaires et traitements"),
    ("FR", "Charges sociales"),
    ("FS", "Dotations aux amortissements sur immobilisations"),
    ("FT", "Dotations aux dépréciations sur immobilisations"),
    ("FU", "Dotations aux dépréciations sur actif circulant"),
    ("FV", "Dotations aux provisions"),
    ("FW", "Autres charges"),
    ("FX", "TOTAL DES CHARGES D'EXPLOITATION (II)"),
    ("FY", "RÉSULTAT D'EXPLOITATION (I-II)"),
    ("FZ", "Bénéfice attribué ou perte transférée (III)"),
    ("GA", "Perte supportée ou bénéfice transféré (IV)"),
];

/// Codes du formulaire 2053 (COMPTE DE RÉSULTAT - II)
const CODES_2053: &[(&str, &str)] = &[
    ("GB", "Produits financiers de participations"),
    ("GC", "Produits des autres valeurs mobilières"),
    ("GD", "Autres intérêts et produits assimilés"),
    ("GE", "Reprises sur provisions et transferts de charges"),
    ("GF", "Différences positives de change"),
    ("GG", "Produits nets sur cessions de VMP"),
    ("GH", "TOTAL DES PRODUITS FINANCIERS (V)"),
    ("GI", "Dotations financières aux amort. et provisions"),
    ("GJ", "Intérêts et charges assimilées"),
    ("GK", "Différences négatives de change"),
    ("GL", "Charges nettes sur cessions de VMP"),
    ("GM", "TOTAL DES CHARGES FINANCIÈRES (VI)"),
    ("GN", "RÉSULTAT FINANCIER (V-VI)"),
    ("GO", "RÉSULTAT COURANT AVANT IMPÔTS (I-II+III-IV+V-VI)"),
    ("GP", "Produits exceptionnels sur opérations de gestion"),
    ("GQ", "Produits exceptionnels sur opérations en capital"),
    ("GR", "Reprises sur provisions et transferts de charges"),
    ("GS", "TOTAL DES PRODUITS EXCEPTIONNELS (VII)"),
    ("GT", "Charges exceptionnelles sur opérations de gestion"),
    ("GU", "Charges exceptionnelles sur opérations en capital"),
    ("GV", "Dotations exceptionnelles aux amort. et provisions"),
    ("GW", "TOTAL DES CHARGES EXCEPTIONNELLES (VIII)"),
    ("GX", "RÉSULTAT EXCEPTIONNEL (VII-VIII)"),
    ("GY", "Participation des salariés aux résultats (IX)"),
    ("GZ", "Impôts sur les bénéfices (X)"),
    ("HA", "TOTAL DES PRODUITS (I+III+V+VII)"),
    ("HB", "TOTAL DES CHARGES (II+IV+VI+VIII+IX+X)"),
    ("HC", "BÉNÉFICE OU PERTE"),
];

/// Codes du formulaire 2054 (IMMOBILISATIONS)
const CODES_2054: &[(&str, &str)] = &[
    ("IA", "Frais d'établissement"),
    ("IB", "Frais de développement"),
    ("IC", "Concessions et droits similaires"),
    ("ID", "Fonds commercial"),
    ("IE", "Autres immobilisations incorporelles"),
    ("IF", "Avances et acomptes"),
    ("IG", "TOTAL Immobilisations incorporelles"),
    ("IH", "Terrains"),
    ("II", "Constructions"),
    ("IJ", "Installations techniques, matériel"),
    ("IK", "Autres immobilisations corporelles"),
    ("IL", "Immobilisations en cours"),
    ("IM", "Avances et acomptes"),
    ("IN", "TOTAL Immobilisations corporelles"),
    ("IO", "Participations"),
    ("IP", "Créances rattachées"),
    ("IQ", "Autres titres immobilisés"),
    ("IR", "Prêts"),
    ("IS", "Autres"),
    ("IT", "TOTAL Immobilisations financières"),
    ("IU", "TOTAL GÉNÉRAL"),
];

/// Codes du formulaire 2055 (AMORTISSEMENTS)
const CODES_2055: &[(&str, &str)] = &[
    ("JA", "Frais d'établissement"),
    ("JB", "Frais de développement"),
    ("JC", "Concessions et droits similaires"),
    ("JD", "Autres immobilisations incorporelles"),
    ("JE", "TOTAL Immobilisations incorporelles"),
    ("JF", "Terrains"),
    ("JG", "Constructions"),
    ("JH", "Installations techniques"),
    ("JI", "Autres immobilisations corporelles"),
    ("JJ", "TOTAL Immobilisations corporelles"),
    ("JK", "TOTAL GÉNÉRAL"),
];

/// Codes du formulaire 2056 (PROVISIONS)
const CODES_2056: &[(&str, &str)] = &[
    ("KA", "Provisions réglementées"),
    ("KB", "Provisions pour risques et charges"),
    ("KC", "Provisions pour dépréciation"),
    ("KD", "TOTAL"),
];

/// Codes du formulaire 2057 (ÉCHÉANCES CRÉANCES ET DETTES)
const CODES_2057: &[(&str, &str)] = &[
    ("LA", "De l'actif immobilisé - Créances rattachées"),
    ("LB", "Prêts"),
    ("LC", "Autres immobilisations financières"),
    ("LD", "De l'actif circulant - Clients et comptes rattachés"),
    ("LE", "Autres créances"),
    ("LF", "Capital souscrit, appelé, non versé"),
    ("LG", "TOTAL Créances"),
    ("LH", "Emprunts obligataires convertibles"),
    ("LI", "Autres emprunts obligataires"),
    ("LJ", "Emprunts et dettes établissements de crédit"),
    ("LK", "Emprunts et dettes financières diverses"),
    ("LL", "Avances et acomptes reçus"),
    ("LM", "Dettes fournisseurs"),
    ("LN", "Dettes fiscales et sociales"),
    ("LO", "Dettes sur immobilisations"),
    ("LP", "Autres dettes"),
    ("LQ", "TOTAL Dettes"),
];

/// Codes du formulaire 2058-A (DÉTERMINATION DU RÉSULTAT FISCAL)
const CODES_2058A: &[(&str, &str)] = &[
    ("WA", "Résultat comptable - Bénéfice"),
    ("WB", "Résultat comptable - Perte"),
    ("WC", "Réintégrations diverses"),
    ("WD", "Charges non déductibles"),
    ("WE", "Amortissements excédentaires"),
    ("WF", "Provisions non déductibles"),
    ("WG", "Impôt sur les sociétés"),
    ("WH", "Rémunérations excessives"),
    ("WI", "Charges à payer non déductibles"),
    ("WJ", "TOTAL des réintégrations"),
    ("WK", "Produits non imposables"),
    ("WL", "Plus-values à long terme"),
    ("WM", "Quote-part de frais et charges"),
    ("WN", "Provisions antérieurement taxées"),
    ("WO", "Autres déductions"),
    ("WP", "TOTAL des déductions"),
    ("WQ", "Résultat fiscal - Bénéfice"),
    ("WR", "Résultat fiscal - Déficit"),
    ("XI", "Déficits antérieurs reportés"),
    ("XJ", "Bénéfice fiscal"),
    ("XK", "Déficit fiscal"),
    // Codes supplémentaires 2058-A
    ("SJ", "Bénéfice comptable"),
    ("SK", "Perte comptable"),
    ("WZ", "Réintégrations - Amortissements non déductibles"),
    ("XA", "Réintégrations - Provisions non déductibles"),
    ("XB", "Réintégrations - Impôt sur les sociétés"),
    ("XC", "Réintégrations - Charges non déductibles"),
    ("XD", "Déductions - Quote-part frais et charges"),
    ("XE", "Déductions - Produits nets de participations"),
    ("XF", "Déductions - Autres"),
    ("XG", "Résultat fiscal avant imputation déficits"),
    ("XH", "Déficits imputés"),
    ("XN", "Résultat soumis à l'IS"),
    ("YA", "Base imposable au taux normal"),
    ("YB", "Base imposable au taux réduit"),
    ("YC", "Crédit d'impôt recherche"),
    ("YD", "Autres crédits d'impôt"),
    ("YE", "IS à payer"),
    ("YF", "Contribution sociale"),
    ("YG", "Total IS et contributions"),
    ("YH", "Acomptes versés"),
    ("YI", "Solde à payer"),
    ("YJ", "Excédent d'IS"),
    ("YK", "Report déficitaire"),
    ("YL", "Déficit reportable"),
    ("YM", "Plus-values nettes à long terme"),
    ("YN", "Provisions pour investissement"),
    ("YO", "Écarts de réévaluation"),
    ("YP", "Subventions d'équipement"),
    ("ZA", "Résultat de l'exercice"),
    ("ZT", "Quote-part de frais et charges sur dividendes"),
    ("ZV", "Résultat net imposable"),
    ("ZW", "Résultat comptable de l'exercice"),
    ("ZX", "Total à reporter"),
    ("ZY", "Déficit de l'exercice"),
    ("ZZ", "Bénéfice imposable"),
];

/// Codes du formulaire 2058-B (DÉFICITS, INDEMNITÉS CONGÉS)
const CODES_2058B: &[(&str, &str)] = &[
    ("XL", "Déficit de l'exercice"),
    ("XM", "Déficits antérieurs"),
    ("XN", "TOTAL des déficits"),
    ("XO", "Déficits imputés"),
    ("XP", "Déficits restant à reporter"),
    ("YA", "Indemnités pour congés à payer"),
    ("YB", "Charges sociales"),
    ("YC", "Charges fiscales"),
    ("YD", "TOTAL indemnités congés"),
    ("YE", "Congés précédemment déduits"),
    ("YF", "Montant à réintégrer"),
    ("YG", "Montant à déduire"),
];

/// Codes du formulaire 2058-C (AFFECTATION DU RÉSULTAT)
const CODES_2058C: &[(&str, &str)] = &[
    ("ZA", "Résultat de l'exercice"),
    ("ZB", "Report à nouveau antérieur"),
    ("ZC", "Prélèvements sur réserves"),
    ("ZD", "TOTAL à affecter"),
    ("ZE", "Affectations aux réserves"),
    ("ZF", "Dividendes"),
    ("ZG", "Report à nouveau"),
    ("ZH", "Autres affectations"),
    ("ZI", "TOTAL affecté"),
];

/// Codes du formulaire 2059-A (PLUS ET MOINS-VALUES)
const CODES_2059A: &[(&str, &str)] = &[
    ("MA", "Plus-values à court terme"),
    ("MB", "Plus-values à long terme"),
    ("MC", "Moins-values à court terme"),
    ("MD", "Moins-values à long terme"),
    ("ME", "Plus-value nette à court terme"),
    ("MF", "Moins-value nette à court terme"),
    ("MG", "Plus-value nette à long terme"),
    ("MH", "Moins-value nette à long terme"),
];

/// Codes du formulaire 2059-E (VALEUR AJOUTÉE)
const CODES_2059E: &[(&str, &str)] = &[
    ("NA", "Chiffre d'affaires"),
    ("NB", "Variation des stocks"),
    ("NC", "Production immobilisée"),
    ("ND", "Subventions d'exploitation"),
    ("NE", "Autres produits d'exploitation"),
    ("NF", "TOTAL Produits"),
    ("NG", "Achats"),
    ("NH", "Variation de stocks"),
    ("NI", "Autres achats et charges externes"),
    ("NJ", "TOTAL Charges"),
    ("NK", "VALEUR AJOUTÉE"),
];

/// Construit le dictionnaire complet des codes CERFA
fn construire_dictionnaire_codes() -> HashMap<String, HashMap<String, &'static str>> {
    let mut dict = HashMap::new();

    let formulaires: &[(&str, &[(&str, &str)])] = &[
        ("2050", CODES_2050),
        ("2050-AMORT", CODES_2050_AMORT),
        ("2051", CODES_2051),
        ("2052", CODES_2052),
        ("2053", CODES_2053),
        ("2054", CODES_2054),
        ("2055", CODES_2055),
        ("2056", CODES_2056),
        ("2057", CODES_2057),
        ("2058-A", CODES_2058A),
        ("2058-B", CODES_2058B),
        ("2058-C", CODES_2058C),
        ("2059-A", CODES_2059A),
        ("2059-E", CODES_2059E),
    ];

    for (form, codes) in formulaires {
        let mut form_dict = HashMap::new();
        for (code, libelle) in *codes {
            form_dict.insert(code.to_string(), *libelle);
        }
        dict.insert(form.to_string(), form_dict);
    }

    dict
}

/// Formulaires attendus dans une liasse fiscale standard
const FORMULAIRES_ATTENDUS: &[&str] = &[
    "2050", "2051", "2052", "2053", "2054", "2055", "2056", "2057",
    "2058-A", "2058-B", "2058-C",
    "2059-A", "2059-B", "2059-C", "2059-D", "2059-E", "2059-F", "2059-G",
];

/// Noms des formulaires pour l'affichage
fn nom_formulaire(code: &str) -> &'static str {
    match code {
        "2050" => "BILAN - ACTIF",
        "2051" => "BILAN - PASSIF",
        "2052" => "COMPTE DE RESULTAT DE L'EXERCICE (I)",
        "2053" => "COMPTE DE RESULTAT DE L'EXERCICE (II)",
        "2054" => "IMMOBILISATIONS",
        "2055" => "AMORTISSEMENTS",
        "2056" => "PROVISIONS",
        "2057" => "ETAT DES ECHEANCES DES CREANCES ET DES DETTES",
        "2058-A" => "DETERMINATION DU RESULTAT FISCAL",
        "2058-B" => "DEFICITS, INDEMNITES POUR CONGES A PAYER",
        "2058-C" => "TABLEAU D'AFFECTATION DU RESULTAT ET RENSEIGNEMENTS DIVERS",
        "2059-A" => "DETERMINATION DES PLUS ET MOINS-VALUES",
        "2059-B" => "AFFECTATION DES PLUS-VALUES A COURT TERME",
        "2059-C" => "SUIVI DES MOINS-VALUES A LONG TERME",
        "2059-D" => "RESERVE SPECIALE DES PLUS-VALUES A LONG TERME",
        "2059-E" => "DETERMINATION DE LA VALEUR AJOUTEE",
        "2059-F" => "COMPOSITION DU CAPITAL SOCIAL",
        "2059-G" => "FILIALES ET PARTICIPATIONS",
        _ => "FORMULAIRE",
    }
}

/// Resolve a path relative to the working directory
fn resolve_path(working_dir: &str, path: &str) -> PathBuf {
    let path = Path::new(path);
    if path.is_absolute() {
        path.to_path_buf()
    } else {
        Path::new(working_dir).join(path)
    }
}

/// Détecte si le contenu contient des sections OCR cassées (codes et valeurs désalignés)
fn detecter_format_ocr_casse(contenu: &str) -> bool {
    // Pattern: lignes avec juste un code CERFA de 2 lettres (AA, AB, BK, etc.)
    // Exclure les codes courants qui ne sont pas des codes CERFA (SA, SN, SC, etc.)
    let re_code_seul = Regex::new(r"(?m)^\s*([A-Z]{2})\s*$").unwrap();

    // Pattern: format CERFA normal (code CERFA + libellé substantiel + valeur)
    // Le code CERFA doit être suivi d'un libellé commençant par une minuscule ou majuscule accentuée
    // Ex: "AA  Capital souscrit non appelé      150000"
    let re_format_cerfa_normal = Regex::new(
        r"(?m)^([A-HJ-Z][A-Z])\s+[A-Za-zÀ-ÿ][a-zà-ÿ].{15,}(?:\s{2,}|\t)[\d\s,.-]+"
    ).unwrap();

    let codes_seuls: Vec<_> = re_code_seul.find_iter(contenu).collect();
    let lignes_cerfa_normales: Vec<_> = re_format_cerfa_normal.find_iter(contenu).collect();

    // Si beaucoup de codes CERFA isolés (>50) avec un ratio élevé par rapport aux lignes normales
    // Un fichier avec 485 codes isolés et peu de lignes normales est clairement un OCR cassé
    let ratio = if lignes_cerfa_normales.is_empty() {
        codes_seuls.len() as f64
    } else {
        codes_seuls.len() as f64 / lignes_cerfa_normales.len() as f64
    };

    // Condition: au moins 50 codes isolés ET ratio > 10:1
    codes_seuls.len() > 50 && ratio > 10.0
}

/// Extrait les codes isolés d'une section
fn extraire_codes_isoles(section: &str) -> Vec<String> {
    let re = Regex::new(r"(?m)^\s*([A-Z]{2})\s*$").unwrap();
    re.captures_iter(section)
        .filter_map(|cap| cap.get(1).map(|m| m.as_str().to_string()))
        .collect()
}

/// Extrait les valeurs numériques isolées d'une section
/// Retourne les valeurs nettoyées et formatées
fn extraire_valeurs_isolees(section: &str) -> Vec<String> {
    // Pattern: ligne avec une ou plusieurs valeurs numériques séparées par des espaces
    // Peut contenir des espaces comme séparateurs de milliers (ex: "2 534 377")
    let re = Regex::new(r"(?m)^\s*([\d\s]+)\s*$").unwrap();
    let mut valeurs = Vec::new();

    for cap in re.captures_iter(section) {
        if let Some(m) = cap.get(1) {
            let ligne = m.as_str().trim();

            // Parser la ligne pour extraire les valeurs individuelles
            // Les valeurs sont séparées par 2+ espaces, les milliers par 1 espace
            let mut current_val = String::new();
            let mut last_was_digit = false;
            let mut space_count = 0;

            for c in ligne.chars() {
                if c.is_ascii_digit() {
                    if space_count >= 2 && !current_val.is_empty() {
                        // Nouvelle valeur détectée
                        let val_clean = current_val.trim().to_string();
                        if is_valid_financial_value(&val_clean) {
                            valeurs.push(val_clean);
                        }
                        current_val = String::new();
                    }
                    current_val.push(c);
                    last_was_digit = true;
                    space_count = 0;
                } else if c == ' ' {
                    if last_was_digit {
                        space_count += 1;
                        if space_count == 1 {
                            current_val.push(' '); // Séparateur de milliers
                        }
                    }
                }
            }

            // Dernière valeur de la ligne
            if !current_val.is_empty() {
                let val_clean = current_val.trim().to_string();
                if is_valid_financial_value(&val_clean) {
                    valeurs.push(val_clean);
                }
            }
        }
    }
    valeurs
}

/// Vérifie si une valeur est une valeur financière valide (pas une date/année)
fn is_valid_financial_value(val: &str) -> bool {
    let digits_only: String = val.chars().filter(|c| c.is_ascii_digit()).collect();
    if digits_only.is_empty() {
        return false;
    }

    // Ignorer les années (2020-2030) et petits nombres isolés
    if let Ok(n) = digits_only.parse::<i64>() {
        // Valeur valide si > 100 ou si contient des espaces (séparateur milliers)
        n > 100 || val.contains(' ')
    } else {
        true
    }
}

pub struct FormatLiasseFiscaleTool {
    working_dir: String,
}

impl FormatLiasseFiscaleTool {
    pub fn new(working_dir: String) -> Self {
        Self { working_dir }
    }

    /// Reconstruit une liasse depuis un OCR cassé
    fn reconstruire_depuis_ocr(&self, contenu: &str) -> String {
        let dict = construire_dictionnaire_codes();
        let mut resultat = String::new();
        let separateur = "═".repeat(80);
        let separateur_fin = "─".repeat(80);

        // En-tête
        resultat.push_str(&format!("{}\n", separateur));
        resultat.push_str("                    LIASSE FISCALE FORMATEE (OCR RECONSTRUIT)\n");
        resultat.push_str(&format!("{}\n\n", separateur));

        // Détecter les formulaires présents
        let formulaires_trouves = self.detecter_formulaires(contenu);

        if formulaires_trouves.is_empty() {
            resultat.push_str("Aucun formulaire détecté dans le document.\n");
            return resultat;
        }

        // Traiter chaque formulaire
        for (i, (pos, code_form)) in formulaires_trouves.iter().enumerate() {
            let debut = *pos;
            let fin = if i + 1 < formulaires_trouves.len() {
                formulaires_trouves[i + 1].0
            } else {
                contenu.len()
            };

            let section = &contenu[debut..fin];

            // Extraire codes et valeurs de la section
            let codes = extraire_codes_isoles(section);
            let valeurs = extraire_valeurs_isolees(section);

            // En-tête du formulaire
            resultat.push_str(&format!("\n{}\n", separateur));
            resultat.push_str(&format!("  FORMULAIRE {} - {}\n", code_form, nom_formulaire(code_form)));
            resultat.push_str(&format!("{}\n\n", separateur));

            // Associer codes et valeurs
            if !codes.is_empty() {
                let nb_lignes = codes.len().min(valeurs.len().max(1));

                for idx in 0..nb_lignes {
                    let code = codes.get(idx).map(|s| s.as_str()).unwrap_or("??");
                    let valeur = valeurs.get(idx).map(|s| s.as_str()).unwrap_or("");

                    // Chercher le libellé dans TOUS les dictionnaires
                    // (l'OCR peut mélanger les codes de différents formulaires)
                    let libelle = dict.values()
                        .find_map(|d| d.get(code).copied())
                        .unwrap_or("");

                    // Formater la ligne
                    let libelle_tronque = if libelle.len() > 45 {
                        &libelle[..45]
                    } else {
                        libelle
                    };

                    resultat.push_str(&format!(
                        "  {:3} │ {:<45} │ {:>15}\n",
                        code,
                        libelle_tronque,
                        valeur
                    ));
                }
            } else {
                // Pas de codes extraits, garder le contenu brut formaté
                resultat.push_str(&self.formater_section(section, code_form));
            }

            resultat.push_str(&format!("\n{}\n", separateur_fin));
        }

        resultat
    }

    /// Détecte les formulaires présents dans le texte
    fn detecter_formulaires(&self, contenu: &str) -> Vec<(usize, String)> {
        let mut formulaires = Vec::new();

        // Patterns pour détecter les numéros de formulaire
        // Ex: "2050", "N° 2050", "CERFA 2050", "Formulaire 2050", "DGFiP N° 2050"
        let patterns = [
            r"(?i)(?:N[°o]?\s*|CERFA\s*|DGFiP\s*N[°o]?\s*|Formulaire\s*)?(205[0-7]|2058-[A-C]|2059-[A-G])",
            r"(?i)\b(205[0-7])\b",
            r"(?i)\b(2058[\s-]?[A-C])\b",
            r"(?i)\b(2059[\s-]?[A-G])\b",
        ];

        for pattern in patterns {
            if let Ok(re) = Regex::new(pattern) {
                for cap in re.captures_iter(contenu) {
                    if let Some(m) = cap.get(1) {
                        let code = m.as_str().to_uppercase().replace(" ", "-");
                        let pos = m.start();

                        // Éviter les doublons proches (même formulaire dans les 100 premiers caractères)
                        let already_found = formulaires.iter().any(|(p, c): &(usize, String)| {
                            c == &code && (*p as i64 - pos as i64).abs() < 100
                        });

                        if !already_found {
                            formulaires.push((pos, code));
                        }
                    }
                }
            }
        }

        // Trier par position
        formulaires.sort_by_key(|(pos, _)| *pos);

        // Dédupliquer en gardant la première occurrence de chaque formulaire
        let mut seen = HashSet::new();
        formulaires.retain(|(_, code)| seen.insert(code.clone()));

        formulaires
    }

    /// Formate le contenu avec des séparations claires
    fn formater_contenu(&self, contenu: &str, formulaires: &[(usize, String)]) -> String {
        if formulaires.is_empty() {
            return contenu.to_string();
        }

        let mut resultat = String::new();
        let separateur = "═".repeat(80);
        let separateur_fin = "─".repeat(80);

        // En-tête
        resultat.push_str(&format!("{}\n", separateur));
        resultat.push_str("                    LIASSE FISCALE FORMATEE\n");
        resultat.push_str(&format!("{}\n\n", separateur));

        // Découper le contenu par formulaire
        for (i, (pos, code)) in formulaires.iter().enumerate() {
            let debut = *pos;
            let fin = if i + 1 < formulaires.len() {
                formulaires[i + 1].0
            } else {
                contenu.len()
            };

            // Extraire la section
            let section = &contenu[debut..fin];

            // En-tête du formulaire
            resultat.push_str(&format!("\n{}\n", separateur));
            resultat.push_str(&format!("  FORMULAIRE {} - {}\n", code, nom_formulaire(code)));
            resultat.push_str(&format!("{}\n\n", separateur));

            // Contenu formaté
            let section_formatee = self.formater_section(section, code);
            resultat.push_str(&section_formatee);

            resultat.push_str(&format!("\n{}\n", separateur_fin));
        }

        resultat
    }

    /// Formate une section individuelle (un formulaire)
    fn formater_section(&self, section: &str, _code: &str) -> String {
        let mut lignes_formatees = Vec::new();

        // Pattern pour détecter les codes de ligne (ex: AA, AB, AC, BK, etc.)
        // suivis de leur libellé et valeur
        let re_code_ligne = Regex::new(r"(?m)^([A-Z]{2})\s+(.+?)(?:\s{2,}|\t)([\d\s,.-]+)?\s*$").ok();

        for ligne in section.lines() {
            let ligne_trimmed = ligne.trim();

            // Ignorer les lignes vides consécutives
            if ligne_trimmed.is_empty() {
                if !lignes_formatees.last().map(|l: &String| l.is_empty()).unwrap_or(false) {
                    lignes_formatees.push(String::new());
                }
                continue;
            }

            // Essayer de formater les lignes avec code
            if let Some(ref re) = re_code_ligne {
                if let Some(cap) = re.captures(ligne_trimmed) {
                    let code = cap.get(1).map(|m| m.as_str()).unwrap_or("");
                    let libelle = cap.get(2).map(|m| m.as_str()).unwrap_or("");
                    let valeur = cap.get(3).map(|m| m.as_str()).unwrap_or("");

                    // Formatage aligné
                    let ligne_fmt = format!("  {:3} │ {:<50} │ {:>15}",
                        code,
                        if libelle.len() > 50 { &libelle[..50] } else { libelle },
                        valeur.trim()
                    );
                    lignes_formatees.push(ligne_fmt);
                    continue;
                }
            }

            // Sinon, garder la ligne telle quelle avec indentation
            lignes_formatees.push(format!("  {}", ligne_trimmed));
        }

        lignes_formatees.join("\n")
    }

    /// Vérifie les formulaires manquants
    fn verifier_formulaires(&self, trouves: &[(usize, String)]) -> Vec<String> {
        let codes_trouves: HashSet<_> = trouves.iter().map(|(_, c)| c.as_str()).collect();

        FORMULAIRES_ATTENDUS
            .iter()
            .filter(|&code| !codes_trouves.contains(code))
            .map(|s| s.to_string())
            .collect()
    }
}

#[async_trait]
impl Tool for FormatLiasseFiscaleTool {
    fn name(&self) -> &str {
        "format_liasse_fiscale"
    }

    fn description(&self) -> &str {
        "Reformater un fichier texte de liasse fiscale. Ajoute des séparations entre formulaires (2050-2059) et structure les codes/libellés."
    }

    fn parameters_schema(&self) -> serde_json::Value {
        json!({
            "type": "object",
            "properties": {
                "input": {
                    "type": "string",
                    "description": "Chemin du fichier texte brut de la liasse fiscale"
                },
                "output": {
                    "type": "string",
                    "description": "Chemin du fichier de sortie formaté (optionnel, défaut: input_formatted.txt)"
                },
                "verifier": {
                    "type": "boolean",
                    "description": "Vérifier la présence de tous les formulaires attendus (défaut: true)"
                }
            },
            "required": ["input"]
        })
    }

    async fn execute(&self, args: &ParsedToolCall) -> Result<ToolResult> {
        let input = args
            .get_string("input")
            .ok_or_else(|| anyhow::anyhow!("Paramètre 'input' manquant"))?;

        let verifier = args.get_bool("verifier").unwrap_or(true);

        let input_path = resolve_path(&self.working_dir, &input);

        // Chemin de sortie par défaut
        let output = args.get_string("output").unwrap_or_else(|| {
            let stem = input_path.file_stem().and_then(|s| s.to_str()).unwrap_or("liasse");
            let parent = input_path.parent().unwrap_or(Path::new("."));
            parent.join(format!("{}_formatted.txt", stem)).to_string_lossy().to_string()
        });
        let output_path = resolve_path(&self.working_dir, &output);

        // Vérifier que le fichier existe
        if !input_path.exists() {
            return Ok(ToolResult::error(format!("Fichier introuvable: {}", input_path.display())));
        }

        // Lire le contenu
        let contenu = fs::read_to_string(&input_path)
            .await
            .map_err(|e| anyhow::anyhow!("Échec de lecture: {}", e))?;

        // Détecter si c'est un OCR cassé
        let est_ocr_casse = detecter_format_ocr_casse(&contenu);

        // Détecter les formulaires
        let formulaires = self.detecter_formulaires(&contenu);

        // Formater le contenu selon le type de format
        let contenu_formate = if est_ocr_casse {
            // Mode reconstruction OCR
            self.reconstruire_depuis_ocr(&contenu)
        } else {
            // Mode formatage standard
            self.formater_contenu(&contenu, &formulaires)
        };

        // Vérifier les formulaires manquants
        let manquants = if verifier {
            self.verifier_formulaires(&formulaires)
        } else {
            Vec::new()
        };

        // Écrire le fichier formaté
        if let Some(parent) = output_path.parent() {
            fs::create_dir_all(parent).await?;
        }
        fs::write(&output_path, &contenu_formate).await?;

        // Construire le message de résultat
        let mode_info = if est_ocr_casse {
            " (mode reconstruction OCR)"
        } else {
            ""
        };

        let mut message = format!(
            "Liasse formatée{}: {}\n\nFormulaires détectés ({}):\n",
            mode_info,
            output_path.display(),
            formulaires.len()
        );

        for (_, code) in &formulaires {
            message.push_str(&format!("  ✓ {} - {}\n", code, nom_formulaire(code)));
        }

        if !manquants.is_empty() {
            message.push_str(&format!("\n⚠ Formulaires non détectés ({}):\n", manquants.len()));
            for code in &manquants {
                message.push_str(&format!("  ✗ {} - {}\n", code, nom_formulaire(code)));
            }
        }

        message.push_str(&format!(
            "\nStatistiques:\n  - Lignes: {}\n  - Caractères: {}",
            contenu_formate.lines().count(),
            contenu_formate.len()
        ));

        Ok(ToolResult::success(message))
    }

    fn requires_confirmation(&self) -> bool {
        true
    }

    fn summarize_call(&self, args: &ParsedToolCall) -> String {
        let input = args.get_string("input").unwrap_or_else(|| "?".to_string());
        format!("Formater la liasse fiscale: {}", input)
    }
}
