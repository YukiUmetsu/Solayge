use serde_json::{json, Value};

use crate::models::PermissionProfile;

/// Hard rails that apply to every profile. Policies never prompt and cannot be
/// lifted by `--auto` or an "Allow always" approval.
fn rails() -> Vec<Value> {
    vec![
        json!({ "action": "permission", "resource": "shell:sudo *", "effect": "deny" }),
        json!({ "action": "permission", "resource": "shell:rm -rf /*", "effect": "deny" }),
        json!({ "action": "permission", "resource": "shell:rm -fr /*", "effect": "deny" }),
        json!({ "action": "permission", "resource": "shell:git push --force*", "effect": "deny" }),
        json!({ "action": "permission", "resource": "shell:git push -f*", "effect": "deny" }),
        json!({ "action": "permission", "resource": "read:*/.ssh/*", "effect": "deny" }),
    ]
}

fn permissions(profile: PermissionProfile) -> Vec<Value> {
    match profile {
        PermissionProfile::Autonomous => vec![
            json!({ "action": "*", "resource": "*", "effect": "allow" }),
            // The one exception: reaching outside the project folder asks, so
            // access is confirmed in-app instead of silently allowed (and then
            // announced by the OS). The later, more specific rule wins.
            json!({ "action": "external_directory", "resource": "*", "effect": "ask" }),
        ],
        PermissionProfile::Supervised => vec![
            json!({ "action": "read", "resource": "*", "effect": "allow" }),
            json!({ "action": "glob", "resource": "*", "effect": "allow" }),
            json!({ "action": "grep", "resource": "*", "effect": "allow" }),
            json!({ "action": "edit", "resource": "*", "effect": "allow" }),
            json!({ "action": "shell", "resource": "*", "effect": "ask" }),
            json!({ "action": "shell", "resource": "git status *", "effect": "allow" }),
            json!({ "action": "shell", "resource": "git diff *", "effect": "allow" }),
            json!({ "action": "shell", "resource": "git log *", "effect": "allow" }),
            json!({ "action": "shell", "resource": "git show *", "effect": "allow" }),
            json!({ "action": "webfetch", "resource": "*", "effect": "ask" }),
            json!({ "action": "websearch", "resource": "*", "effect": "ask" }),
            json!({ "action": "external_directory", "resource": "*", "effect": "ask" }),
        ],
        PermissionProfile::Readonly => vec![
            json!({ "action": "read", "resource": "*", "effect": "allow" }),
            json!({ "action": "glob", "resource": "*", "effect": "allow" }),
            json!({ "action": "grep", "resource": "*", "effect": "allow" }),
            json!({ "action": "edit", "resource": "*", "effect": "deny" }),
            json!({ "action": "shell", "resource": "*", "effect": "deny" }),
            json!({ "action": "execute", "resource": "*", "effect": "deny" }),
            json!({ "action": "webfetch", "resource": "*", "effect": "allow" }),
            json!({ "action": "websearch", "resource": "*", "effect": "allow" }),
            json!({ "action": "external_directory", "resource": "*", "effect": "deny" }),
        ],
    }
}

pub fn config_for(profile: PermissionProfile) -> Value {
    json!({
        "permissions": permissions(profile),
        "experimental": { "policies": rails() },
    })
}

pub fn config_json(profile: PermissionProfile) -> String {
    config_for(profile).to_string()
}

/// Whether the profile should run with `--auto` (auto-approve non-denied asks).
pub fn uses_auto(profile: PermissionProfile) -> bool {
    matches!(profile, PermissionProfile::Autonomous)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn has_rule(cfg: &Value, action: &str, resource: &str, effect: &str) -> bool {
        cfg["permissions"]
            .as_array()
            .unwrap()
            .iter()
            .any(|r| r["action"] == action && r["resource"] == resource && r["effect"] == effect)
    }

    fn has_policy(cfg: &Value, resource: &str) -> bool {
        cfg["experimental"]["policies"]
            .as_array()
            .unwrap()
            .iter()
            .any(|r| r["resource"] == resource && r["effect"] == "deny")
    }

    #[test]
    fn every_profile_carries_the_rails() {
        for p in [
            PermissionProfile::Autonomous,
            PermissionProfile::Supervised,
            PermissionProfile::Readonly,
        ] {
            let cfg = config_for(p);
            assert!(has_policy(&cfg, "shell:sudo *"), "{p:?} missing sudo rail");
            assert!(
                has_policy(&cfg, "shell:git push --force*"),
                "{p:?} missing force-push rail"
            );
            assert!(has_policy(&cfg, "read:*/.ssh/*"), "{p:?} missing ssh rail");
        }
    }

    #[test]
    fn readonly_denies_edits_and_shell() {
        let cfg = config_for(PermissionProfile::Readonly);
        assert!(has_rule(&cfg, "edit", "*", "deny"));
        assert!(has_rule(&cfg, "shell", "*", "deny"));
        assert!(has_rule(&cfg, "execute", "*", "deny"));
        assert!(has_rule(&cfg, "read", "*", "allow"));
    }

    #[test]
    fn supervised_asks_for_shell_but_allows_git_reads() {
        let cfg = config_for(PermissionProfile::Supervised);
        assert!(has_rule(&cfg, "shell", "*", "ask"));
        assert!(has_rule(&cfg, "shell", "git status *", "allow"));
        assert!(has_rule(&cfg, "edit", "*", "allow"));
    }

    #[test]
    fn autonomous_allows_everything_and_uses_auto() {
        let cfg = config_for(PermissionProfile::Autonomous);
        assert!(has_rule(&cfg, "*", "*", "allow"));
        // The lone exception: outside folders ask rather than being silently
        // allowed, so the app can confirm the directory and purpose.
        assert!(has_rule(&cfg, "external_directory", "*", "ask"));
        assert!(uses_auto(PermissionProfile::Autonomous));
        assert!(!uses_auto(PermissionProfile::Supervised));
        assert!(!uses_auto(PermissionProfile::Readonly));
    }
}
