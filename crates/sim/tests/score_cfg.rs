use project1990_sim::ScoreConfig;

const SCORE_CFG: &str = include_str!("../../../data/score.cfg");

#[test]
fn data_score_cfg_loads() {
    let cfg = ScoreConfig::parse(SCORE_CFG).expect("data/score.cfg");
    assert_eq!(cfg.enemy_basic, 100);
    assert_eq!(cfg.enemy_armor, 200);
    assert_eq!(cfg.enemy_boss, 400);
    assert_eq!(cfg.streak_bonus_per_step, 50);
    assert_eq!(cfg.clear_bonus, 500);
}
