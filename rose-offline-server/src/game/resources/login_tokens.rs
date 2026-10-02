use std::time::{Duration, Instant};

use bevy::{ecs::prelude::Entity, prelude::Resource};

/// How long a token may exist without any connected client before it counts as
/// abandoned. Covers the hand-over from the login server to the world server,
/// where the login connection closes before the world connection claims the token.
const UNCLAIMED_TOKEN_TIMEOUT: Duration = Duration::from_secs(30);

pub struct LoginToken {
    pub username: String,
    pub token: u32,
    pub selected_world_server: Entity,
    pub selected_game_server: Entity,
    pub selected_character: String,
    pub login_client: Option<Entity>,
    pub world_client: Option<Entity>,
    pub game_client: Option<Entity>,
    pub created: Instant,
}

#[derive(Default, Resource)]
pub struct LoginTokens {
    pub tokens: Vec<LoginToken>,
}

impl LoginTokens {
    pub fn new() -> Self {
        Default::default()
    }

    pub fn generate(
        &mut self,
        username: String,
        login_client: Entity,
        selected_world_server: Entity,
        selected_game_server: Entity,
    ) -> u32 {
        let mut token = 0u32;
        while token == 0 || self.tokens.iter().any(|x| x.token == token) {
            token = rand::random();
        }
        self.tokens.push(LoginToken {
            username,
            token,
            selected_world_server,
            selected_game_server,
            selected_character: String::default(),
            login_client: Some(login_client),
            world_client: None,
            game_client: None,
            created: Instant::now(),
        });
        token
    }

    pub fn find_username_token(&self, username: &str) -> Option<&LoginToken> {
        self.tokens.iter().find(|token| token.username == username)
    }

    pub fn get_token_mut(&mut self, token_id: u32) -> Option<&mut LoginToken> {
        self.tokens.iter_mut().find(|token| token.token == token_id)
    }

    /// Detaches a disconnected login client from its tokens. The tokens are kept:
    /// the client closes the login connection before it claims the token on the
    /// world server.
    pub fn remove_login_client(&mut self, entity: Entity) {
        for login_token in self.tokens.iter_mut() {
            if login_token.login_client == Some(entity) {
                login_token.login_client = None;
            }
        }
    }

    /// Detaches a disconnected world client from its token, removing the token
    /// when no game client uses it either.
    pub fn remove_world_client(&mut self, entity: Entity) {
        self.tokens.retain_mut(|login_token| {
            if login_token.world_client != Some(entity) {
                return true;
            }

            login_token.world_client = None;
            login_token.game_client.is_some()
        });
    }

    /// Detaches a disconnected game client from its token, removing the token
    /// when no world client uses it either.
    pub fn remove_game_client(&mut self, entity: Entity) {
        self.tokens.retain_mut(|login_token| {
            if login_token.game_client != Some(entity) {
                return true;
            }

            login_token.game_client = None;
            login_token.world_client.is_some()
        });
    }

    /// Removes tokens that no client has used for longer than the hand-over
    /// timeout, e.g. when the client never reached the world server. Such tokens
    /// would otherwise block the account with `AlreadyLoggedIn` forever.
    pub fn remove_abandoned_tokens(&mut self) {
        self.tokens.retain(|login_token| {
            login_token.login_client.is_some()
                || login_token.world_client.is_some()
                || login_token.game_client.is_some()
                || login_token.created.elapsed() < UNCLAIMED_TOKEN_TIMEOUT
        });
    }
}
