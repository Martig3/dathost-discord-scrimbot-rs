use std::collections::HashMap;
use std::time::Duration;

use async_std::task;
use rand::Rng;
use regex::Regex;
use reqwest::header;
use serde::{Deserialize, Serialize};
use serenity::client::Context;
use serenity::model::channel::{Message, ReactionType};
use serenity::model::guild::{Guild, GuildContainer};
use serenity::model::id::EmojiId;
use serenity::model::user::User;
use serenity::utils::MessageBuilder;

use crate::{BotState, Config, Draft, Maps, QueueMessages, ReadyQueue, State, StateContainer, SteamIdCache, TeamNameCache, UserQueue};

struct ReactionResult {
    count: u64,
    map: String,
}

#[allow(non_snake_case)]
#[derive(Serialize, Deserialize)]
struct Stats {
    steamId: String,
    totalKills: f64,
    totalDeaths: f64,
    totalAssists: f64,
    kdRatio: f64,
    map: String,
    hs: f64,
    rws: f64,
    adr: f64,
    rating: f64,
    playCount: i64,
    winPercentage: f64,
}

pub(crate) async fn handle_join(context: &Context, msg: &Message, author: &User) {
    let mut data = context.data.write().await;
    let steam_id_cache: &HashMap<u64, String> = &data.get::<SteamIdCache>().unwrap();
    if !steam_id_cache.contains_key(author.id.as_u64()) {
        let response = MessageBuilder::new()
            .mention(author)
            .push(" steamID not found for your discord user, \
                    please use `.steamid <your steamID>` to assign one. Example: `.steamid STEAM_0:1:12345678` ")
            .push("https://steamid.io/ is an easy way to find your steamID for your account")
            .build();
        if let Err(why) = msg.channel_id.say(&context.http, &response).await {
            eprintln!("Error sending message: {:?}", why);
        }
        return;
    }
    let user_queue: &mut Vec<User> = &mut data.get_mut::<UserQueue>().unwrap();
    if user_queue.len() >= 10 {
        let response = MessageBuilder::new()
            .mention(author)
            .push(" sorry but the queue is full.")
            .build();
        if let Err(why) = msg.channel_id.say(&context.http, &response).await {
            eprintln!("Error sending message: {:?}", why);
        }
        return;
    }
    if user_queue.contains(&author) {
        let response = MessageBuilder::new()
            .mention(author)
            .push(" is already in the queue.")
            .build();
        if let Err(why) = msg.channel_id.say(&context.http, &response).await {
            eprintln!("Error sending message: {:?}", why);
        }
        return;
    }
    user_queue.push(author.clone());
    let response = MessageBuilder::new()
        .mention(author)
        .push(" has been added to the queue. Queue size: ")
        .push(user_queue.len().to_string())
        .push("/10")
        .build();
    if let Err(why) = msg.channel_id.say(&context.http, &response).await {
        eprintln!("Error sending message: {:?}", why);
    }
    let queued_msgs: &mut HashMap<u64, String> = data.get_mut::<QueueMessages>().unwrap();
    let quote_regex = Regex::new("[\"”“](.*?)[\"”“]").unwrap();
    if let Some(mat) = quote_regex.find(&msg.content) {
        let start = mat.start();
        let mut end = mat.end();
        end = end.min(start + 50);
        queued_msgs.insert(*msg.author.id.as_u64(), String::from(msg.content[start..end].trim()));
    }
    let config: &Config = data.get::<Config>().unwrap();
    if let Some(role_id) = config.discord.assign_role_id {
        if let Ok(value) = msg.author.has_role(&context.http, msg.guild_id.unwrap(), role_id).await {
            if !value {
                let guild = Guild::get(&context.http, msg.guild_id.unwrap()).await.unwrap();
                if let Ok(mut member) = guild.member(&context.http, msg.author.id).await {
                    if let Err(err) = member.add_role(&context.http, role_id).await {
                        eprintln!("assign_role_id exists but cannot add role to user, check bot permissions");
                        eprintln!("{:?}", err);
                    }
                }
            }
        }
    }
}

pub(crate) async fn handle_leave(context: Context, msg: Message) {
    let mut data = context.data.write().await;
    let state: &mut StateContainer = data.get_mut::<BotState>().unwrap();
    if state.state != State::Queue {
        send_simple_tagged_msg(&context, &msg, " cannot `.leave` the queue after `.start`, use `.cancel` to start over if needed.", &msg.author).await;
        return;
    }
    let user_queue: &mut Vec<User> = data.get_mut::<UserQueue>().unwrap();
    if !user_queue.contains(&msg.author) {
        let response = MessageBuilder::new()
            .mention(&msg.author)
            .push(" is not in the queue. Type `.join` to join the queue.")
            .build();
        if let Err(why) = msg.channel_id.say(&context.http, &response).await {
            eprintln!("Error sending message: {:?}", why);
        }
        return;
    }
    let index = user_queue.iter().position(|r| r.id == msg.author.id).unwrap();
    user_queue.remove(index);
    let response = MessageBuilder::new()
        .mention(&msg.author)
        .push(" has left the queue. Queue size: ")
        .push(user_queue.len().to_string())
        .push("/10")
        .build();
    if let Err(why) = msg.channel_id.say(&context.http, &response).await {
        eprintln!("Error sending message: {:?}", why);
    }
    let queued_msgs: &mut HashMap<u64, String> = data.get_mut::<QueueMessages>().unwrap();
    if queued_msgs.get(&msg.author.id.as_u64()).is_some() {
        queued_msgs.remove(&msg.author.id.as_u64());
    }
}

pub(crate) async fn handle_list(context: Context, msg: Message) {
    let data = context.data.write().await;
    let user_queue: &Vec<User> = data.get::<UserQueue>().unwrap();
    let queue_msgs: &HashMap<u64, String> = data.get::<QueueMessages>().unwrap();
    let mut user_name = String::new();
    for u in user_queue {
        user_name.push_str(format!("\n- @{}", u.name).as_str());
        if let Some(value) = queue_msgs.get(u.id.as_u64()) {
            user_name.push_str(format!(": `{}`", value).as_str());
        }
    }
    let response = MessageBuilder::new()
        .push("Current queue size: ")
        .push(&user_queue.len())
        .push("/10")
        .push(user_name)
        .build();

    if let Err(why) = msg.channel_id.say(&context.http, &response).await {
        eprintln!("Error sending message: {:?}", why);
    }
}

pub(crate) async fn handle_clear(context: Context, msg: Message) {
    if !admin_check(&context, &msg, true).await { return; }
    let mut data = context.data.write().await;
    let user_queue: &mut Vec<User> = &mut data.get_mut::<UserQueue>().unwrap();
    user_queue.clear();
    let response = MessageBuilder::new()
        .mention(&msg.author)
        .push(" cleared queue")
        .build();
    if let Err(why) = msg.channel_id.say(&context.http, &response).await {
        eprintln!("Error sending message: {:?}", why);
    }
}

pub(crate) async fn handle_help(context: Context, msg: Message) {
    let mut commands = String::from("
**Commands:**
`.join` - Join the queue, add an optional message in quotes (max 50 characters) i.e. `.join \"available at 9pm\"`
`.leave` - Leave the queue
`.list` - List all users in the queue
`.steamid` - Set your steamID i.e. `.steamid STEAM_0:1:12345678`
`.maps` - Lists all maps in available for play
`.stats` - Lists all available statistics for user. Add `Xm` to display past X months where X is a single digit integer. Add `.top10` to display top 10 ranking with an optional `.top10 Xm` month filter.
`.teamname` - Sets a custom team name when you are a captain i.e. `.teamname TeamName`

_These are commands used during the `.start` process:_
`.captain` - Add yourself as a captain.
`.pick` - If you are a captain, this is used to pick a player
`.ready` - After the draft phase is completed, use this to ready up
`.unready` - After the draft phase is completed, use this to cancel your `.ready` status
`.readylist` - Lists players not readied up
");
    let admin_commands = String::from("
_These are admin commands:_
`.start` - Start the match setup process
`.kick` - Kick a player by mentioning them i.e. `.kick @user`
`.addmap` - Add a map to the map vote i.e. `.addmap de_dust2` _Note: map must be present on the server or the server will not start._
`.removemap` - Remove a map from the map vote i.e. `.removemap de_dust2`
`.recoverqueue` - Manually set a queue, tag all users to add after the command
`.clear` - Clear the queue
`.cancel` - Cancels `.start` process
    ");
    if admin_check(&context, &msg, false).await {
        commands.push_str(&admin_commands)
    }
    let response = MessageBuilder::new()
        .push(commands)
        .build();
    if let Ok(channel) = &msg.author.create_dm_channel(&context.http).await {
        if let Err(why) = channel.say(&context.http, &response).await {
            eprintln!("Error sending message: {:?}", why);
        }
    } else {
        eprintln!("Error sending .help dm");
    }
}

pub(crate) async fn handle_recover_queue(context: Context, msg: Message) {
    if !admin_check(&context, &msg, true).await { return; }
    {
        let mut data = context.data.write().await;
        let user_queue: &mut Vec<User> = &mut data.get_mut::<UserQueue>().unwrap();
        user_queue.clear();
        let queue_msgs: &mut HashMap<u64, String> = &mut data.get_mut::<QueueMessages>().unwrap();
        queue_msgs.clear();
    }
    for mention in &msg.mentions {
        handle_join(&context, &msg, &mention).await
    }
}

pub(crate) async fn handle_ready_list(context: Context, msg: Message) {
    let data = context.data.write().await;
    let ready_queue: &Vec<User> = data.get::<ReadyQueue>().unwrap();
    let user_queue: &Vec<User> = data.get::<UserQueue>().unwrap();
    let user_name: String = user_queue
        .iter()
        .filter(|user| !ready_queue.contains(user))
        .map(|user| format!("\n- @{}", user.name))
        .collect();
    let response = MessageBuilder::new()
        .push("Players that are not `.ready`:")
        .push(user_name)
        .build();

    if let Err(why) = msg.channel_id.say(&context.http, &response).await {
        eprintln!("Error sending message: {:?}", why);
    }
}

pub(crate) async fn handle_start(context: Context, msg: Message) {
    let admin_check = admin_check(&context, &msg, true).await;
    if !admin_check { return; }
    let mut data = context.data.write().await;
    let bot_state: &StateContainer = data.get::<BotState>().unwrap();
    if bot_state.state != State::Queue {
        send_simple_tagged_msg(&context, &msg, " `.start` command has already been entered", &msg.author).await;
        return;
    }
    let user_queue: &mut Vec<User> = data.get_mut::<UserQueue>().unwrap();
    if !user_queue.contains(&msg.author) && !admin_check {
        send_simple_tagged_msg(&context, &msg, " non-admin users that are not in the queue cannot start the match", &msg.author).await;
        return;
    }
    if user_queue.len() != 10 {
        let response = MessageBuilder::new()
            .mention(&msg.author)
            .push(" the queue is not full yet")
            .build();
        if let Err(why) = msg.channel_id.say(&context.http, &response).await {
            eprintln!("Error sending message: {:?}", why);
        }
        return;
    }
    let user_queue_mention: String = user_queue
        .iter()
        .map(|user| format!("- <@{}>\n", user.id))
        .collect();
    let response = MessageBuilder::new()
        .push(user_queue_mention)
        .push_bold_line("Scrim setup is starting...")
        .build();
    if let Err(why) = msg.channel_id.say(&context.http, &response).await {
        eprintln!("Error sending message: {:?}", why);
    }
    let bot_state: &mut StateContainer = data.get_mut::<BotState>().unwrap();
    bot_state.state = State::MapPick;
    let maps: &Vec<String> = &data.get::<Maps>().unwrap();
    let mut unicode_to_maps: HashMap<String, String> = HashMap::new();
    let a_to_z = ('a'..'z').collect::<Vec<_>>();
    let unicode_emoji_map = populate_unicode_emojis().await;
    for (i, map) in maps.iter().enumerate() {
        unicode_to_maps.insert(String::from(unicode_emoji_map.get(&a_to_z[i]).unwrap()), String::from(map));
    }
    let emoji_suffixes = a_to_z[..maps.len()].to_vec();
    let vote_text: String = emoji_suffixes
        .iter()
        .enumerate()
        .map(|(i, c)| format!(":regional_indicator_{}: `{}`\n", c, &maps[i]))
        .collect();
    let response = MessageBuilder::new()
        .push_bold_line("Map Vote:")
        .push(vote_text)
        .build();
    let vote_msg = msg.channel_id.say(&context.http, &response).await.unwrap();
    for c in emoji_suffixes {
        vote_msg.react(&context.http, ReactionType::Unicode(String::from(unicode_emoji_map.get(&c).unwrap()))).await.unwrap();
    }
    task::sleep(Duration::from_secs(50)).await;
    let response = MessageBuilder::new()
        .push("Voting will end in 10 seconds")
        .build();
    if let Err(why) = msg.channel_id.say(&context.http, &response).await {
        eprintln!("Error sending message: {:?}", why);
    }
    task::sleep(Duration::from_secs(10)).await;
    let updated_vote_msg = vote_msg.channel_id.message(&context.http, vote_msg.id).await.unwrap();
    let mut results: Vec<ReactionResult> = Vec::new();
    for reaction in updated_vote_msg.reactions {
        let react_as_map: Option<&String> = unicode_to_maps.get(reaction.reaction_type.to_string().as_str());
        if react_as_map != None {
            let map = String::from(react_as_map.unwrap());
            results.push(ReactionResult {
                count: reaction.count,
                map,
            });
        }
    }
    let max_count = results
        .iter()
        .max_by(|x, y| x.count.cmp(&y.count))
        .unwrap()
        .count;
    let final_results: Vec<ReactionResult> = results
        .into_iter()
        .filter(|m| m.count == max_count)
        .collect();
    let mut selected_map = String::from("");
    if final_results.len() > 1 {
        let map = &final_results.get(rand::thread_rng().gen_range(0, final_results.len())).unwrap().map;
        let response = MessageBuilder::new()
            .push("Maps were tied, `")
            .push(&map)
            .push("` was selected at random")
            .build();
        if let Err(why) = msg.channel_id.say(&context.http, &response).await {
            eprintln!("Error sending message: {:?}", why);
        }
        selected_map.push_str(map);
    } else {
        let map = &final_results[0].map;
        let response = MessageBuilder::new()
            .push("Map vote has concluded. `")
            .push(&map)
            .push("` will be played")
            .build();
        if let Err(why) = msg.channel_id.say(&context.http, &response).await {
            eprintln!("Error sending message: {:?}", why);
        }
        selected_map.push_str(map);
    }
    let config: &Config = data.get::<Config>().unwrap();
    let client = reqwest::Client::new();
    let dathost_username = &config.dathost.username;
    let dathost_password: Option<String> = Some(String::from(&config.dathost.password));
    let update_map_url = format!("https://dathost.net/api/0.1/game-servers/{}", &config.server.id);
    let resp = client
        .put(&update_map_url)
        .form(&[("csgo_settings.mapgroup_start_map", &selected_map)])
        .basic_auth(&dathost_username, dathost_password)
        .send()
        .await
        .unwrap();
    println!("Change map response - {:#?}", resp.status());
    let mut bot_state: &mut StateContainer = data.get_mut::<BotState>().unwrap();
    bot_state.state = State::CaptainPick;
    let draft: &mut Draft = &mut data.get_mut::<Draft>().unwrap();
    draft.captain_a = None;
    draft.captain_b = None;
    draft.team_a = Vec::new();
    draft.team_b = Vec::new();
    send_simple_msg(&context, &msg, "Starting captain pick phase. Two users type `.captain` to start picking teams.").await;
}


pub(crate) async fn handle_captain(context: Context, msg: Message) {
    let mut data = context.data.write().await;
    let bot_state: &mut StateContainer = &mut data.get_mut::<BotState>().unwrap();
    if bot_state.state != State::CaptainPick {
        send_simple_tagged_msg(&context, &msg, " command ignored, not in the captain pick phase", &msg.author).await;
        return;
    }
    let user_queue: &Vec<User> = data.get::<UserQueue>().unwrap();
    if !user_queue.contains(&msg.author) {
        send_simple_tagged_msg(&context, &msg, " command ignored, you are not in the queue", &msg.author).await;
        return;
    }
    let draft: &mut Draft = &mut data.get_mut::<Draft>().unwrap();
    if draft.captain_a != None && &msg.author == draft.captain_a.as_ref().unwrap() {
        send_simple_tagged_msg(&context, &msg, " you're already a captain!", &msg.author).await;
        return;
    }
    if draft.captain_a == None {
        send_simple_tagged_msg(&context, &msg, " is set as captain.", &msg.author).await;
        draft.captain_a = Some(msg.author.clone());
    } else {
        send_simple_tagged_msg(&context, &msg, " is set as captain.", &msg.author).await;
        draft.captain_b = Some(msg.author.clone());
    }
    if draft.captain_a != None && draft.captain_b != None {
        send_simple_msg(&context, &msg, "Randomizing captain pick order...").await;
        // flip a coin, if 1 switch captains
        if rand::thread_rng().gen_range(0, 2) != 0 {
            let captain_a = draft.captain_a.clone();
            let captain_b = draft.captain_b.clone();
            draft.captain_a = captain_b;
            draft.captain_b = captain_a;
        }
        draft.team_a.push(draft.captain_a.clone().unwrap());
        draft.team_b.push(draft.captain_b.clone().unwrap());
        send_simple_tagged_msg(&context, &msg, " is set as the first pick captain (Team A)", &draft.captain_a.clone().unwrap()).await;
        send_simple_tagged_msg(&context, &msg, " is set as the second captain (Team B)", &draft.captain_b.clone().unwrap()).await;
        draft.current_picker = draft.captain_a.clone();
        let response = MessageBuilder::new()
            .push("Captain pick has concluded. Starting draft phase. ")
            .mention(&draft.current_picker.clone().unwrap())
            .push(" gets first `.pick @<user>`")
            .build();
        if let Err(why) = msg.channel_id.say(&context.http, &response).await {
            eprintln!("Error sending message: {:?}", why);
        }
        let bot_state: &mut StateContainer = &mut data.get_mut::<BotState>().unwrap();
        bot_state.state = State::Draft;
        let user_queue: &Vec<User> = &mut data.get::<UserQueue>().unwrap();
        let draft: &Draft = &mut data.get::<Draft>().unwrap();
        let teamname_cache = data.get::<TeamNameCache>().unwrap();
        let team_a_name = teamname_cache.get(draft.captain_a.as_ref().unwrap().id.as_u64())
            .unwrap_or(&draft.captain_a.as_ref().unwrap().name);
        let team_b_name = teamname_cache.get(draft.captain_b.as_ref().unwrap().id.as_u64())
            .unwrap_or(&draft.captain_b.as_ref().unwrap().name);
        list_unpicked(&user_queue, &draft, &context, &msg, team_a_name, team_b_name).await;
    }
}

pub(crate) async fn handle_pick(context: Context, msg: Message) {
    let mut data = context.data.write().await;
    let bot_state: &mut StateContainer = &mut data.get_mut::<BotState>().unwrap();
    if bot_state.state != State::Draft {
        send_simple_tagged_msg(&context, &msg, " it is not currently the draft phase", &msg.author).await;
        return;
    }
    if msg.mentions.is_empty() {
        send_simple_tagged_msg(&context, &msg, " please mention a discord user in the message", &msg.author).await;
        return;
    }
    let picked = msg.mentions[0].clone();
    let user_queue: &Vec<User> = &data.get::<UserQueue>().unwrap().to_vec();
    if !user_queue.contains(&picked) {
        send_simple_tagged_msg(&context, &msg, " this user is not in the queue", &msg.author).await;
        return;
    }
    let draft = data.get::<Draft>().unwrap();
    let current_picker = draft.current_picker.clone().unwrap();
    if msg.author != *draft.captain_a.as_ref().unwrap() && msg.author != *draft.captain_b.as_ref().unwrap() {
        send_simple_tagged_msg(&context, &msg, " you are not a captain", &msg.author).await;
        return;
    }
    if current_picker != msg.author {
        send_simple_tagged_msg(&context, &msg, " it is not your turn to pick", &msg.author).await;
        return;
    }
    if msg.mentions.is_empty() {
        send_simple_tagged_msg(&context, &msg, " please mention a discord user in your message.", &msg.author).await;
        return;
    }
    if draft.team_a.contains(&picked) || draft.team_b.contains(&picked) {
        send_simple_tagged_msg(&context, &msg, " this player is already on a team", &msg.author).await;
        return;
    }

    let teamname_cache = data.get::<TeamNameCache>().unwrap();
    let team_a_name = String::from(teamname_cache.get(draft.captain_a.as_ref().unwrap().id.as_u64())
        .unwrap_or(&draft.captain_a.as_ref().unwrap().name));
    let team_b_name = String::from(teamname_cache.get(draft.captain_b.as_ref().unwrap().id.as_u64())
        .unwrap_or(&draft.captain_b.as_ref().unwrap().name));
    let draft: &mut Draft = &mut data.get_mut::<Draft>().unwrap();
    if draft.captain_a.as_ref().unwrap() == &current_picker {
        send_simple_tagged_msg(&context, &msg, &format!(" has been added to Team {}", team_a_name), &picked).await;
        draft.team_a.push(picked);
        draft.current_picker = draft.captain_b.clone();
        list_unpicked(&user_queue, &draft, &context, &msg, &team_a_name, &team_b_name).await;
    } else {
        send_simple_tagged_msg(&context, &msg, &format!(" has been added to Team {}", team_b_name), &picked).await;
        draft.team_b.push(picked);
        draft.current_picker = draft.captain_a.clone();
        list_unpicked(&user_queue, &draft, &context, &msg, &team_a_name, &team_b_name).await;
    }
    if draft.team_a.len() == 5 && draft.team_b.len() == 5 {
        let captain_b = draft.captain_b.clone().unwrap();
        let bot_state: &mut StateContainer = &mut data.get_mut::<BotState>().unwrap();
        bot_state.state = State::SidePick;
        let sidepick_msg = send_simple_tagged_msg(&context, &msg, " type `.ct` or `.t` to pick a starting side.", &captain_b).await;
        let config: &mut Config = &mut data.get_mut::<Config>().unwrap();
        if let Some(msg) = sidepick_msg {
            if let Some(emote_ct_id) = &config.discord.emote_ct_id {
                if let Some(emote_ct_name) = &config.discord.emote_ct_name {
                    if let Err(why) = msg.react(&context.http, ReactionType::Custom { animated: false, id: EmojiId(*emote_ct_id), name: Some(String::from(emote_ct_name)) }).await {
                        eprintln!("Error reacting with custom emoji: {:?}", why)
                    };
                }
            }
            if let Some(emote_t_id) = &config.discord.emote_t_id {
                if let Some(emote_t_name) = &config.discord.emote_t_name {
                    if let Err(why) = msg.react(&context.http, ReactionType::Custom { animated: false, id: EmojiId(*emote_t_id), name: Some(String::from(emote_t_name)) }).await {
                        eprintln!("Error reacting with custom emoji: {:?}", why)
                    };
                }
            }
        }
    }
}

pub(crate) async fn list_unpicked(user_queue: &Vec<User>, draft: &Draft, context: &Context, msg: &Message, team_a_name: &String, team_b_name: &String) {
    let remaining_users: String = user_queue
        .iter()
        .filter(|user| !draft.team_a.contains(user) && !draft.team_b.contains(user))
        .map(|user| format!("- @{}\n", &user.name))
        .collect();
    let team_a: String = draft.team_a
        .iter()
        .map(|user| format!("- @{}\n", &user.name))
        .collect();
    let team_b: String = draft.team_b
        .iter()
        .map(|user| format!("- @{}\n", &user.name))
        .collect();
    let response = MessageBuilder::new()
        .push_bold_line(format!("Team {}:", team_a_name))
        .push_line(team_a)
        .push_bold_line(format!("Team {}:", team_b_name))
        .push_line(team_b)
        .push_bold_line("Remaining players: ")
        .push_line(remaining_users)
        .build();

    if let Err(why) = msg.channel_id.say(&context.http, &response).await {
        eprintln!("Error sending message: {:?}", why);
    }
}

pub(crate) async fn handle_ct_option(context: Context, msg: Message) {
    let mut data = context.data.write().await;
    let bot_state: &mut StateContainer = &mut data.get_mut::<BotState>().unwrap();
    if bot_state.state != State::SidePick {
        send_simple_tagged_msg(&context, &msg, " it is not currently the side pick phase", &msg.author).await;
        return;
    }
    let draft: &mut Draft = &mut data.get_mut::<Draft>().unwrap();
    if &msg.author != draft.captain_b.as_ref().unwrap() {
        send_simple_tagged_msg(&context, &msg, " you are not Captain B", &msg.author).await;
        return;
    }
    draft.team_b_start_side = String::from("ct");
    let bot_state: &mut StateContainer = &mut data.get_mut::<BotState>().unwrap();
    bot_state.state = State::Ready;
    send_simple_msg(&context, &msg, "Setup is completed. Type `.ready` when you are able start playing. This is a final ready check, once all players are `.ready` the server and match will immediately start.").await;
}

pub(crate) async fn handle_t_option(context: Context, msg: Message) {
    let mut data = context.data.write().await;
    let bot_state: &mut StateContainer = &mut data.get_mut::<BotState>().unwrap();
    if bot_state.state != State::SidePick {
        send_simple_tagged_msg(&context, &msg, " it is not currently the side pick phase", &msg.author).await;
        return;
    }
    let draft: &mut Draft = &mut data.get_mut::<Draft>().unwrap();
    if &msg.author != draft.captain_b.as_ref().unwrap() {
        send_simple_tagged_msg(&context, &msg, " you are not Captain B", &msg.author).await;
        return;
    }
    draft.team_b_start_side = String::from("t");
    let bot_state: &mut StateContainer = &mut data.get_mut::<BotState>().unwrap();
    bot_state.state = State::Ready;
    send_simple_msg(&context, &msg, "Setup is completed. Type `.ready` when you are able start playing. This is a final ready check, once all players are `.ready` the server and match will immediately start.").await;
}

pub(crate) async fn handle_steam_id(context: Context, msg: Message) {
    let mut data = context.data.write().await;
    let steam_id_cache: &mut HashMap<u64, String> = &mut data.get_mut::<SteamIdCache>().unwrap();
    let split_content = msg.content.trim().split(' ').take(2).collect::<Vec<_>>();
    if split_content.len() == 1 {
        send_simple_tagged_msg(&context, &msg, " please check the command formatting. There must be a space in between `.steamid` and your steamid. \
        Example: `.steamid STEAM_0:1:12345678`", &msg.author).await;
        return;
    }
    let steam_id_str: String = String::from(split_content[1]);
    let steam_id_regex = Regex::new("^STEAM_[0-5]:[01]:\\d+$").unwrap();
    if !steam_id_regex.is_match(&steam_id_str) {
        send_simple_tagged_msg(&context, &msg, " invalid steamid formatting. Please follow this example: `.steamid STEAM_0:1:12345678`", &msg.author).await;
        return;
    }
    steam_id_cache.insert(*msg.author.id.as_u64(), String::from(&steam_id_str));
    write_to_file(String::from("steam-ids.json"), serde_json::to_string(steam_id_cache).unwrap()).await;
    let response = MessageBuilder::new()
        .push("Updated steamid for ")
        .mention(&msg.author)
        .push(" to `")
        .push(&steam_id_str)
        .push("`")
        .build();
    if let Err(why) = msg.channel_id.say(&context.http, &response).await {
        eprintln!("Error sending message: {:?}", why);
    }
}

pub(crate) async fn handle_map_list(context: Context, msg: Message) {
    let data = context.data.write().await;
    let maps: &Vec<String> = data.get::<Maps>().unwrap();
    let map_str: String = maps.iter().map(|map| format!("- `{}`\n", map)).collect();
    let response = MessageBuilder::new()
        .push_line("Current map pool:")
        .push(map_str)
        .build();
    if let Err(why) = msg.channel_id.say(&context.http, &response).await {
        eprintln!("Error sending message: {:?}", why);
    }
}

pub(crate) async fn handle_kick(context: Context, msg: Message) {
    if !admin_check(&context, &msg, true).await { return; }
    let mut data = context.data.write().await;
    let state: &mut StateContainer = data.get_mut::<BotState>().unwrap();
    if state.state != State::Queue {
        send_simple_tagged_msg(&context, &msg, " cannot `.kick` the queue after `.start`, use `.cancel` to start over if needed.", &msg.author).await;
        return;
    }
    let user_queue: &mut Vec<User> = data.get_mut::<UserQueue>().unwrap();
    let user = &msg.mentions[0];
    if !user_queue.contains(&user) {
        let response = MessageBuilder::new()
            .mention(&msg.author)
            .push(" is not in the queue.")
            .build();
        if let Err(why) = msg.channel_id.say(&context.http, &response).await {
            eprintln!("Error sending message: {:?}", why);
        }
        return;
    }
    let index = user_queue.iter().position(|r| r.id == user.id).unwrap();
    user_queue.remove(index);
    let response = MessageBuilder::new()
        .mention(user)
        .push(" has been kicked. Queue size: ")
        .push(user_queue.len().to_string())
        .push("/10")
        .build();
    if let Err(why) = msg.channel_id.say(&context.http, &response).await {
        eprintln!("Error sending message: {:?}", why);
    }
}

pub(crate) async fn handle_add_map(context: Context, msg: Message) {
    if !admin_check(&context, &msg, true).await { return; }
    let mut data = context.data.write().await;
    let maps: &mut Vec<String> = data.get_mut::<Maps>().unwrap();
    if maps.len() >= 26 {
        let response = MessageBuilder::new()
            .mention(&msg.author)
            .push(" unable to add map, max amount reached.")
            .build();
        if let Err(why) = msg.channel_id.say(&context.http, &response).await {
            eprintln!("Error sending message: {:?}", why);
        }
        return;
    }
    let map_name: String = String::from(msg.content.trim().split(" ").take(2).collect::<Vec<_>>()[1]);
    if maps.contains(&map_name) {
        let response = MessageBuilder::new()
            .mention(&msg.author)
            .push(" unable to add map, already exists.")
            .build();
        if let Err(why) = msg.channel_id.say(&context.http, &response).await {
            eprintln!("Error sending message: {:?}", why);
        }
        return;
    }
    maps.push(String::from(&map_name));
    write_to_file(String::from("maps.json"), serde_json::to_string(maps).unwrap()).await;
    let response = MessageBuilder::new()
        .mention(&msg.author)
        .push(" added map: `")
        .push(&map_name)
        .push("`")
        .build();
    if let Err(why) = msg.channel_id.say(&context.http, &response).await {
        eprintln!("Error sending message: {:?}", why);
    }
}

pub(crate) async fn handle_remove_map(context: Context, msg: Message) {
    if !admin_check(&context, &msg, true).await { return; }
    let mut data = context.data.write().await;
    let maps: &mut Vec<String> = data.get_mut::<Maps>().unwrap();
    let map_name: String = String::from(msg.content.trim().split(" ").take(2).collect::<Vec<_>>()[1]);
    if !maps.contains(&map_name) {
        let response = MessageBuilder::new()
            .mention(&msg.author)
            .push(" this map doesn't exist in the list.")
            .build();
        if let Err(why) = msg.channel_id.say(&context.http, &response).await {
            eprintln!("Error sending message: {:?}", why);
        }
        return;
    }
    let index = maps.iter().position(|m| m == &map_name).unwrap();
    maps.remove(index);
    write_to_file(String::from("maps.json"), serde_json::to_string(maps).unwrap()).await;
    let response = MessageBuilder::new()
        .mention(&msg.author)
        .push(" removed map: `")
        .push(&map_name)
        .push("`")
        .build();
    if let Err(why) = msg.channel_id.say(&context.http, &response).await {
        eprintln!("Error sending message: {:?}", why);
    }
}

pub(crate) async fn handle_unknown(context: Context, msg: Message) {
    let response = MessageBuilder::new()
        .push("Unknown command, type `.help` for list of commands.")
        .build();
    if let Err(why) = msg.channel_id.say(&context.http, &response).await {
        eprintln!("Error sending message: {:?}", why);
    }
}

pub(crate) async fn write_to_file(path: String, content: String) {
    let mut error_string = String::from("Error writing to ");
    error_string.push_str(&path);
    std::fs::write(path, content)
        .expect(&error_string);
}

pub(crate) async fn handle_ready(context: Context, msg: Message) {
    let mut data = context.data.write().await;
    let bot_state: &StateContainer = data.get_mut::<BotState>().unwrap();
    if bot_state.state != State::Ready {
        send_simple_tagged_msg(&context, &msg, " command ignored. The draft has not been completed yet", &msg.author).await;
        return;
    }
    let user_queue: &Vec<User> = &data.get::<UserQueue>().unwrap();
    if !user_queue.contains(&msg.author) {
        send_simple_tagged_msg(&context, &msg, " you are not in the queue.", &msg.author).await;
        return;
    }
    let ready_queue: &mut Vec<User> = &mut data.get_mut::<ReadyQueue>().unwrap();
    if ready_queue.contains(&msg.author) {
        let response = MessageBuilder::new()
            .mention(&msg.author)
            .push(", you're already `.ready`")
            .build();
        if let Err(why) = msg.channel_id.say(&context.http, &response).await {
            eprintln!("Error sending message: {:?}", why);
        }
        return;
    }
    ready_queue.push(msg.author.clone());
    let response = MessageBuilder::new()
        .mention(&msg.author)
        .push(" is ready. Players ready: ")
        .push(ready_queue.len().to_string())
        .push("/10")
        .build();
    if let Err(why) = msg.channel_id.say(&context.http, &response).await {
        eprintln!("Error sending message: {:?}", why);
    }

    if ready_queue.len() >= 10 {
        println!("Launching server...");
        let draft: &Draft = &data.get::<Draft>().unwrap();
        let steam_id_cache: &HashMap<u64, String> = &data.get::<SteamIdCache>().unwrap();
        let mut team_a_steam_ids: Vec<String> = draft.team_a
            .iter()
            .map(|u| steam_id_cache.get(u.id.as_u64()).unwrap().to_string())
            .collect();
        for team_a_steam_id in &mut team_a_steam_ids {
            team_a_steam_id.replace_range(6..7, "1");
        }
        let mut team_a_steam_id_str: String = team_a_steam_ids
            .iter()
            .map(|s| format!("{},", s))
            .collect();
        team_a_steam_id_str = String::from(&team_a_steam_id_str[..team_a_steam_id_str.len() - 1]);
        let mut team_b_steam_ids: Vec<String> = draft.team_b
            .iter()
            .map(|u| steam_id_cache.get(u.id.as_u64()).unwrap().to_string())
            .collect();
        for team_b_steam_id in &mut team_b_steam_ids {
            team_b_steam_id.replace_range(6..7, "1");
        }
        let mut team_b_steam_id_str: String = team_b_steam_ids
            .iter()
            .map(|s| format!("{},", s))
            .collect();
        team_b_steam_id_str = String::from(&team_b_steam_id_str[..team_b_steam_id_str.len() - 1]);
        let response = MessageBuilder::new()
            .push("All players are ready. Server is starting...")
            .build();
        if let Err(why) = msg.channel_id.say(&context.http, &response).await {
            eprintln!("Error sending message: {:?}", why);
        }
        let team_ct: String;
        let team_t: String;
        if draft.team_b_start_side == "ct" {
            team_ct = team_b_steam_id_str;
            team_t = team_a_steam_id_str;
        } else {
            team_ct = team_a_steam_id_str;
            team_t = team_b_steam_id_str;
        }

        println!("Starting server with the following params:");
        println!("team1_steam_ids:'{}'", &team_t);
        println!("team2_steam_ids:'{}'", &team_ct);

        let config: &Config = data.get::<Config>().unwrap();
        let client = reqwest::Client::new();
        let dathost_username = &config.dathost.username;
        let dathost_password: Option<String> = Some(String::from(&config.dathost.password));
        let server_id = &config.server.id;
        let match_end_url = if &config.dathost.match_end_url == &None { "" } else { &config.dathost.match_end_url.as_ref().unwrap() };
        let start_match_url = String::from("https://dathost.net/api/0.1/matches");
        println!("match_end_webhook_url:'{}'", &match_end_url);
        println!("game_server_id:'{}'", &server_id);
        let mut auth_str = config.scrimbot_api_config.scrimbot_api_user.clone().unwrap();
        auth_str.push_str(":");
        auth_str.push_str(&*config.scrimbot_api_config.scrimbot_api_password.clone().unwrap());
        let base64 = base64::encode(auth_str);
        let mut auth_str = String::from("Basic ");
        auth_str.push_str(&base64);
        let resp = client
            .post(&start_match_url)
            .form(&[("game_server_id", &server_id),
                ("team1_steam_ids", &&team_t),
                ("team2_steam_ids", &&team_ct),
                ("enable_pause", &&String::from("true")),
                ("webhook_authorization_header", &&auth_str),
                ("match_end_webhook_url", &&match_end_url.to_string())])
            .basic_auth(&dathost_username, dathost_password)
            .send()
            .await
            .unwrap();
        println!("Start match response code - {}", &resp.status());

        if resp.status().is_success() {
            let steam_web_url: String = format!("steam://connect/{}", &config.server.url);
            let port_start = &config.server.url.find(':').unwrap_or_else(|| 0 as usize) + 1;
            let gotv_port = String::from(&config.server.url[port_start..config.server.url.len()]).parse::<i64>().unwrap_or_else(|_| 0) + 1;
            let gotv_url = format!("{}{}", &config.server.url[0..port_start], gotv_port);
            send_simple_msg(&context, &msg, &format!("Server has started.\n\n**Connection info:**\nLink: {}\nConsole: \
            `connect {}`\n\n_GOTV Info:_\nLink: {}\nConsole: `connect {}`", steam_web_url, &config.server.url, &format!("steam://connect/{}", gotv_url), gotv_url)).await;
        } else {
            send_simple_msg(&context, &msg, &format!("Server failed to start, match POST response code: {}", &resp.status().as_str())).await;
        }
        let draft: &Draft = data.get::<Draft>().unwrap();
        let config: &Config = &data.get::<Config>().unwrap();
        if let Some(team_a_channel_id) = config.discord.team_a_channel_id {
            for user in &draft.team_a {
                if let Some(guild) = &msg.guild(&context.cache).await {
                    if let Err(why) = guild.move_member(&context.http, user.id, team_a_channel_id).await {
                        println!("Cannot move user: {:?}", why);
                    }
                }
            }
        }
        if let Some(team_b_channel_id) = config.discord.team_b_channel_id {
            for user in &draft.team_b {
                if let Some(guild) = &msg.guild(&context.cache).await {
                    if let Err(why) = guild.move_member(&context.http, user.id, team_b_channel_id).await {
                        println!("Cannot move user: {:?}", why);
                    }
                }
            }
        }
        if let Some(teamname_cache) = data.get::<TeamNameCache>() {
            let send_command_url = format!("https://dathost.net/api/0.1/game-servers/{}/console", &config.server.id);
            let dathost_password: Option<String> = Some(String::from(&config.dathost.password));
            let default_team_a_name = &format!("Team {}", &draft.captain_a.as_ref().unwrap().name);
            let default_team_b_name = &format!("Team {}", &draft.captain_b.as_ref().unwrap().name);
            let team_a_name = teamname_cache.get(&draft.captain_a.as_ref().unwrap().id.as_u64())
                .unwrap_or(default_team_a_name);
            let team_b_name = teamname_cache.get(&draft.captain_b.as_ref().unwrap().id.as_u64())
                .unwrap_or(default_team_b_name);
            let team_one_command;
            let team_two_command;
            if draft.team_b_start_side == "ct" {
                team_one_command = format!("mp_teamname_1 {}", &team_b_name);
                team_two_command = format!("mp_teamname_2 {}", &team_a_name);
            } else {
                team_one_command = format!("mp_teamname_1 {}", &team_a_name);
                team_two_command = format!("mp_teamname_2 {}", &team_b_name);
            }
            if let Err(resp) = client
                .post(&send_command_url)
                .form(&[("line", &team_one_command)])
                .basic_auth(&dathost_username, dathost_password)
                .send()
                .await {
                eprintln!("Error setting team name 1: {:?}", resp);
            }
            let dathost_password: Option<String> = Some(String::from(&config.dathost.password));
            if let Err(resp) = client
                .post(&send_command_url)
                .form(&[("line", &team_two_command)])
                .basic_auth(&dathost_username, dathost_password)
                .send()
                .await {
                eprintln!("Error setting team name 2: {:?}", resp);
            }
        }
        if let Some(post_start_msg) = &config.post_setup_msg {
            if let Err(why) = msg.channel_id.say(&context.http, &post_start_msg).await {
                eprintln!("Error sending message: {:?}", why);
            }
        }
        // reset to queue state
        let user_queue: &mut Vec<User> = data.get_mut::<UserQueue>().unwrap();
        user_queue.clear();
        let ready_queue: &mut Vec<User> = data.get_mut::<ReadyQueue>().unwrap();
        ready_queue.clear();
        let draft: &mut Draft = &mut data.get_mut::<Draft>().unwrap();
        draft.team_a = vec![];
        draft.team_b = vec![];
        draft.captain_a = None;
        draft.captain_b = None;
        draft.current_picker = None;
        let bot_state: &mut StateContainer = &mut data.get_mut::<BotState>().unwrap();
        bot_state.state = State::Queue;
        let queue_msgs: &mut HashMap<u64, String> = &mut data.get_mut::<QueueMessages>().unwrap();
        queue_msgs.clear();
    }
}

pub(crate) async fn handle_unready(context: Context, msg: Message) {
    let mut data = context.data.write().await;
    let bot_state: &StateContainer = data.get_mut::<BotState>().unwrap();
    if bot_state.state != State::Ready {
        send_simple_tagged_msg(&context, &msg, " command ignored. The draft has not been completed yet", &msg.author).await;
        return;
    }
    let user_queue: &Vec<User> = &data.get::<UserQueue>().unwrap();
    if !user_queue.contains(&msg.author) {
        send_simple_tagged_msg(&context, &msg, " you are not in the queue.", &msg.author).await;
        return;
    }
    let ready_queue: &mut Vec<User> = &mut data.get_mut::<ReadyQueue>().unwrap();
    let index = ready_queue.iter().position(|r| r.id == msg.author.id).unwrap();
    ready_queue.remove(index);
    send_simple_tagged_msg(&context, &msg, " is no longer `.ready`.", &msg.author).await;
}

pub(crate) async fn handle_cancel(context: Context, msg: Message) {
    if !admin_check(&context, &msg, true).await { return; }
    let mut data = context.data.write().await;
    let bot_state: &StateContainer = &data.get::<BotState>().unwrap();
    if bot_state.state == State::Queue {
        send_simple_tagged_msg(&context, &msg, " command only valid during `.start` process", &msg.author).await;
        return;
    }
    let ready_queue: &mut Vec<User> = data.get_mut::<ReadyQueue>().unwrap();
    ready_queue.clear();
    let draft: &mut Draft = &mut data.get_mut::<Draft>().unwrap();
    draft.team_a = vec![];
    draft.team_b = vec![];
    draft.captain_a = None;
    draft.captain_b = None;
    draft.current_picker = None;
    let bot_state: &mut StateContainer = &mut data.get_mut::<BotState>().unwrap();
    bot_state.state = State::Queue;
    send_simple_tagged_msg(&context, &msg, " `.start` process cancelled.", &msg.author).await;
}

pub(crate) async fn handle_stats(context: Context, msg: Message) {
    let data = context.data.write().await;
    let config: &Config = data.get::<Config>().unwrap();
    if &config.scrimbot_api_config.scrimbot_api_url == &None {
        send_simple_tagged_msg(&context, &msg, " sorry, the scrimbot-api url has not been configured", &msg.author).await;
        return;
    }
    if &config.scrimbot_api_config.scrimbot_api_user == &None || &config.scrimbot_api_config.scrimbot_api_password == &None{
        send_simple_tagged_msg(&context, &msg, " sorry, the scrimbot-api user/password has not been configured", &msg.author).await;
        return;
    }
    if let Some(scrimbot_api_url) = &config.scrimbot_api_config.scrimbot_api_url {
        let mut headers = header::HeaderMap::new();
        let mut auth_str = config.scrimbot_api_config.scrimbot_api_user.clone().unwrap();
        auth_str.push_str(":");
        auth_str.push_str(&*config.scrimbot_api_config.scrimbot_api_password.clone().unwrap());
        let base64 = base64::encode(auth_str);
        let mut auth_str = String::from("Basic ");
        auth_str.push_str(&base64);
        headers.insert("Authorization", auth_str.parse().unwrap());
        let client = reqwest::Client::builder().default_headers(headers).build().unwrap();
        let steam_id_cache: &HashMap<u64, String> = &data.get::<SteamIdCache>().unwrap();
        if steam_id_cache.get(msg.author.id.as_u64()).is_none() {
            send_simple_tagged_msg(&context, &msg, " cannot find your steamId, please assign one using the `.steamid` command", &msg.author).await;
            return;
        }
        let mut steam_id = steam_id_cache.get(msg.author.id.as_u64()).unwrap().clone();
        steam_id.replace_range(6..7, "1");
        let map_idx_start = &msg.content.find("\"");
        let map_idx_end = &msg.content.rfind("\"");
        let mut map_name = String::new();
        if map_idx_start != &None && map_idx_end != &None {
            map_name = String::from(&msg.content[map_idx_start.unwrap() + 1..map_idx_end.unwrap()])
        }
        let split_content = msg.content.trim().split(' ').collect::<Vec<_>>();
        if split_content.len() < 2 || (split_content.len() > 1 && split_content[1].starts_with("\"")) {
            let resp = client
                .get(&format!("{}/api/stats", scrimbot_api_url))
                .query(&[("steamid", &steam_id), (&"map", &map_name)])
                .send()
                .await
                .unwrap();
            if resp.status() != 200 {
                send_simple_tagged_msg(&context, &msg, " sorry, something went wrong retrieving stats", &msg.author).await;
                return;
            }
            let content = resp.text().await.unwrap();
            let stats: Vec<Stats> = serde_json::from_str(&content).unwrap();
            if stats.is_empty() {
                send_simple_tagged_msg(&context, &msg, " sorry, no statistics found", &msg.author).await;
                return;
            }
            let top_ten_str = format_stats(&stats, &context, &steam_id_cache, &msg.guild_id.unwrap().as_u64(), false).await;
            send_simple_tagged_msg(&context, &msg, &top_ten_str, &msg.author).await;
            return;
        }
        let arg_str: String = String::from(split_content[1]);
        let month_regex = Regex::new("\\dm").unwrap();
        if month_regex.is_match(&arg_str) {
            let resp = client
                .get(&format!("{}/api/stats", scrimbot_api_url))
                .query(&[(&"steamid", &steam_id), (&"option", &"range".to_string()), (&"length", &arg_str.get(0..1).unwrap().to_string()), (&"map", &map_name)])
                .send()
                .await
                .unwrap();
            if resp.status() != 200 {
                eprintln!("{}", format!("HTTP error on /api/stats with following params: steamid: {}, option: range, length: {}", &steam_id, &arg_str.get(0..1).unwrap().to_string()));
                return;
            }
            let content = resp.text().await.unwrap();
            let stats: Vec<Stats> = serde_json::from_str(&content).unwrap();
            if stats.is_empty() {
                send_simple_tagged_msg(&context, &msg, " sorry, no statistics found for your discord user (yet!)", &msg.author).await;
                return;
            }
            let top_ten_str = format_stats(&stats, &context, &steam_id_cache, &msg.guild_id.unwrap().as_u64(), false).await;
            send_simple_tagged_msg(&context, &msg, &top_ten_str, &msg.author).await;
            return;
        }
        if &arg_str == "top10" {
            if split_content.len() > 2 && !split_content[2].starts_with("\"") {
                let month_regex = Regex::new("\\dm").unwrap();
                let month_arg = split_content[2];
                if month_regex.is_match(&month_arg) {
                    let resp = client
                        .get(&format!("{}/api/stats", scrimbot_api_url))
                        .query(&[("steamid", &steam_id), ("option", &"top10".to_string()), ("length", &month_arg.get(0..1).unwrap().to_string()), (&"map", &map_name)])
                        .send()
                        .await
                        .unwrap();
                    if resp.status() != 200 {
                        eprintln!("{}", format!("HTTP error on /api/stats with following params: steamid: {}, option: top10", &steam_id));
                        return;
                    }
                    let content = resp.text().await.unwrap();
                    let stats: Vec<Stats> = serde_json::from_str(&content).unwrap();
                    if stats.is_empty() {
                        send_simple_tagged_msg(&context, &msg, " sorry, something went wrong retrieving stats", &msg.author).await;
                        return;
                    }
                    let top_ten_str = format_stats(&stats, &context, &steam_id_cache, &msg.guild_id.unwrap().as_u64(), false).await;
                    if map_name != "" {
                        map_name = format!("`{}`", &map_name)
                    }
                    send_simple_tagged_msg(&context, &msg, &format!(" Top 10 - {} Month(s) {}:\n{}", &month_arg, &map_name, &top_ten_str), &msg.author).await;
                } else {
                    send_simple_tagged_msg(&context, &msg, " month parameter is not properly formatted. Example: `.stats top10 1m`", &msg.author).await;
                }
                return;
            } else {
                let resp = client
                    .get(&format!("{}/api/stats", scrimbot_api_url))
                    .query(&[("steamid", &steam_id), ("option", &"top10".to_string()), (&"map", &map_name)])
                    .send()
                    .await
                    .unwrap();
                if resp.status() != 200 {
                    eprintln!("{}", format!("HTTP error on /api/stats with following params: steamid: {}, option: top10", &steam_id));
                    return;
                }
                let content = resp.text().await.unwrap();
                let stats: Vec<Stats> = serde_json::from_str(&content).unwrap();
                if stats.is_empty() {
                    send_simple_tagged_msg(&context, &msg, " sorry, something went wrong retrieving stats", &msg.author).await;
                    return;
                }
                let top_ten_str = format_stats(&stats, &context, steam_id_cache, msg.guild_id.unwrap().as_u64(), false).await;
                send_simple_tagged_msg(&context, &msg, &format!(" Top 10 Ratio:\n{}", &top_ten_str), &msg.author).await;
                return;
            }
        }
        if &arg_str == "maps" {
            if split_content.len() > 2 && !split_content[2].starts_with("\"") {
                let month_regex = Regex::new("\\dm").unwrap();
                let month_arg = split_content[2];
                if month_regex.is_match(&month_arg) {
                    let resp = client
                        .get(&format!("{}/api/stats", scrimbot_api_url))
                        .query(&[("steamid", &steam_id), ("option", &"maps".to_string()), ("length", &month_arg.get(0..1).unwrap().to_string()), (&"map", &map_name)])
                        .send()
                        .await
                        .unwrap();
                    if resp.status() != 200 {
                        eprintln!("{}", format!("HTTP error on /api/stats with following params: steamid: {}, option: top10", &steam_id));
                        return;
                    }
                    let content = resp.text().await.unwrap();
                    let stats: Vec<Stats> = serde_json::from_str(&content).unwrap();
                    if stats.is_empty() {
                        send_simple_tagged_msg(&context, &msg, " sorry, something went wrong retrieving stats", &msg.author).await;
                        return;
                    }
                    let top_ten_str = format_stats(&stats, &context, &steam_id_cache, &msg.guild_id.unwrap().as_u64(), true).await;
                    send_simple_tagged_msg(&context, &msg, &format!(" Top 10 (per map) - {} Month(s):\n{}", &month_arg, &top_ten_str), &msg.author).await;
                } else {
                    send_simple_tagged_msg(&context, &msg, " month parameter is not properly formatted. Example: `.stats top10 1m`", &msg.author).await;
                }
                return;
            } else {
                let resp = client
                    .get(&format!("{}/api/stats", scrimbot_api_url))
                    .query(&[("steamid", &steam_id), ("option", &"maps".to_string()), (&"map", &map_name)])
                    .send()
                    .await
                    .unwrap();
                if resp.status() != 200 {
                    eprintln!("{}", format!("HTTP error on /api/stats with following params: steamid: {}, option: top10", &steam_id));
                    return;
                }
                let content = resp.text().await.unwrap();
                let stats: Vec<Stats> = serde_json::from_str(&content).unwrap();
                if stats.is_empty() {
                    send_simple_tagged_msg(&context, &msg, " sorry, something went wrong retrieving stats", &msg.author).await;
                    return;
                }
                let top_ten_str = format_stats(&stats, &context, steam_id_cache, msg.guild_id.unwrap().as_u64(), true).await;
                send_simple_tagged_msg(&context, &msg, &format!(" Top 10 (per map):\n{}", &top_ten_str), &msg.author).await;
                return;
            }
        }
    }
}

pub(crate) async fn handle_teamname(context: Context, msg: Message) {
    let mut data = context.data.write().await;
    let teamname_cache: &mut HashMap<u64, String> = &mut data.get_mut::<TeamNameCache>().unwrap();
    let split_content = msg.content.trim().split(' ').collect::<Vec<_>>();
    if split_content.len() < 2 {
        send_simple_tagged_msg(&context, &msg, " invalid message formatting. Example: `.teamname TeamName`", &msg.author).await;
        return;
    }
    let teamname = String::from(&msg.content[10..msg.content.len()]);
    if teamname.len() > 18 {
        send_simple_tagged_msg(&context, &msg, &format!(" team name is over the character limit by {}.", teamname.len() - 18), &msg.author).await;
        return;
    }
    teamname_cache.insert(*msg.author.id.as_u64(), String::from(&teamname));
    write_to_file(String::from("teamnames.json"), serde_json::to_string(teamname_cache).unwrap()).await;
    send_simple_tagged_msg(&context, &msg, &format!(" custom team name successfully set to `{}`", &teamname), &msg.author).await;
}

pub(crate) async fn send_simple_msg(context: &Context, msg: &Message, text: &str) {
    let response = MessageBuilder::new()
        .push(text)
        .build();
    if let Err(why) = msg.channel_id.say(&context.http, &response).await {
        eprintln!("Error sending message: {:?}", why);
    }
}

pub(crate) async fn send_simple_tagged_msg(context: &Context, msg: &Message, text: &str, mentioned: &User) -> Option<Message> {
    let response = MessageBuilder::new()
        .mention(mentioned)
        .push(text)
        .build();
    if let Ok(m) = msg.channel_id.say(&context.http, &response).await {
        Some(m)
    } else {
        eprintln!("Error sending message");
        None
    }
}

pub(crate) async fn admin_check(context: &Context, msg: &Message, print_msg: bool) -> bool {
    let data = context.data.write().await;
    let config: &Config = data.get::<Config>().unwrap();
    let role_name = context.cache.role(msg.guild_id.unwrap(), config.discord.admin_role_id).await.unwrap().name;
    if msg.author.has_role(&context.http, GuildContainer::from(msg.guild_id.unwrap()), config.discord.admin_role_id).await.unwrap_or_else(|_| false) {
        true
    } else {
        if print_msg {
            let response = MessageBuilder::new()
                .mention(&msg.author)
                .push(" this command requires the '")
                .push(role_name)
                .push("' role.")
                .build();
            if let Err(why) = msg.channel_id.say(&context.http, &response).await {
                eprintln!("Error sending message: {:?}", why);
            }
        }
        false
    }
}

async fn format_stats(stats: &Vec<Stats>, context: &Context, steam_id_cache: &HashMap<u64, String>, &guild_id: &u64, print_map: bool) -> String {
    let mut top_ten_str: String = String::from("");
    top_ten_str.push_str("```md\n");
    if stats.len() == 1 {
        let mut map = String::from(stats[0].map.clone());
        if map != "" {
            map = map.replace("de_", "");
            if map.len() > 12 {
                map = map[0..9].to_string();
                map.push_str("...");
            }
            top_ten_str.push_str(&*format!("Map: {:<12} K/D    ADR      RWS     Rating   HS%      Win% (# Games)\n", map));
        } else {
            top_ten_str.push_str("                  K/D    ADR      RWS     Rating   HS%      Win% (# Games)\n");
        }
    } else if !print_map {
        top_ten_str.push_str("     Player       K/D    ADR      RWS     Rating   HS%      Win% (# Games)\n");
    } else {
        top_ten_str.push_str("     Maps         K/D    ADR      RWS     Rating   HS%      Win% (# Games)\n");
    }
    top_ten_str.push_str("-----------------------------------------------------------------------------\n");
    let guild = Guild::get(&context.http, guild_id).await.unwrap();
    let mut count = 0;
    for stat in stats {
        count += 1;
        let user_id: Option<u64> = steam_id_cache.iter()
            .find_map(|(key, val)|
                if &format!("STEAM_1{}", &val[7..]) == &stat.steamId { Some(*key) } else { None }
            );
        let user_cached: Option<User> = context.cache.user(user_id.unwrap_or(0)).await;
        let user: Option<User>;
        if let Some(u) = user_cached {
            user = Some(u)
        } else {
            let member = guild.member(&context.http, user_id.unwrap_or(0)).await;
            if let Ok(m) = member { user = Some(m.user) } else { user = None };
        }
        if let Some(u) = user {
            if !print_map {
                let mut user_name = String::from(u.name.clone());
                if user_name.len() > 12 {
                    user_name = user_name[0..9].to_string();
                    user_name.push_str("...");
                }
                top_ten_str.push_str(&format!("{:>3} @{} {:3.2}  {: >6}   {: >6}   {:3.2}     {:3.1}%    {:3.2}% ({})\n", format!("{}.", count.to_string()), format!("{: <12}", user_name.to_owned()), stat.kdRatio, format!("{:.2}", &stat.adr), format!("{:.2}", &stat.rws), stat.rating, stat.hs, stat.winPercentage, stat.playCount));
            } else {
                let mut map = String::from(stat.map.clone());
                map = map.replace("de_", "");
                if map.len() > 12 {
                    map = map[0..9].to_string();
                    map.push_str("...");
                }
                top_ten_str.push_str(&format!("{:>3}  {} {:3.2}   {: >6}  {: >6}   {:3.2}     {:3.1}%    {:3.2}% ({})\n", format!("{}.", count.to_string()), format!("{: <12}", map.to_owned()), stat.kdRatio, format!("{:.2}", &stat.adr), format!("{:.2}", &stat.rws), stat.rating, stat.hs, stat.winPercentage, stat.playCount))
            }
        } else {
            top_ten_str.push_str(&format!("{:>3} @Error!\n", format!("{}.", count.to_string())))
        };
    }
    top_ten_str.push_str("```");
    return top_ten_str;
}

pub(crate) async fn populate_unicode_emojis() -> HashMap<char, String> {
// I hate this implementation and I deserve to be scolded
// in my defense however, you have to provide unicode emojis to the api
// if Discord's API allowed their shortcuts i.e. ":smile:" instead that would have been more intuitive
    let mut map = HashMap::new();
    map.insert('a', String::from("🇦"));
    map.insert('b', String::from("🇧"));
    map.insert('c', String::from("🇨"));
    map.insert('d', String::from("🇩"));
    map.insert('e', String::from("🇪"));
    map.insert('f', String::from("🇫"));
    map.insert('g', String::from("🇬"));
    map.insert('h', String::from("🇭"));
    map.insert('i', String::from("🇮"));
    map.insert('j', String::from("🇯"));
    map.insert('k', String::from("🇰"));
    map.insert('l', String::from("🇱"));
    map.insert('m', String::from("🇲"));
    map.insert('n', String::from("🇳"));
    map.insert('o', String::from("🇴"));
    map.insert('p', String::from("🇵"));
    map.insert('q', String::from("🇶"));
    map.insert('r', String::from("🇷"));
    map.insert('s', String::from("🇸"));
    map.insert('t', String::from("🇹"));
    map.insert('u', String::from("🇺"));
    map.insert('v', String::from("🇻"));
    map.insert('w', String::from("🇼"));
    map.insert('x', String::from("🇽"));
    map.insert('y', String::from("🇾"));
    map.insert('z', String::from("🇿"));
    map
}
