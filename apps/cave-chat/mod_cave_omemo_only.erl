%%% ZCaveDAO members' chat: a message with words in it travels only end to end
%%% encrypted (OMEMO, either version). In rooms and between members alike,
%%% anything else is refused, with the reason, before it is stored or relayed.
%%% Messages without a body (typing, receipts, markers) pass.
-module(mod_cave_omemo_only).
-behaviour(gen_mod).

-export([start/2, stop/1, reload/3, depends/2, mod_options/1, mod_doc/0]).
-export([muc_filter_message/3, user_send_packet/1]).

-include_lib("xmpp/include/xmpp.hrl").
-include("logger.hrl").

-define(NS_OMEMO2, <<"urn:xmpp:omemo:2">>).
-define(NS_OMEMO, <<"eu.siacs.conversations.axolotl">>).

start(Host, _Opts) ->
    ?INFO_MSG("Only OMEMO-encrypted messages on ~ts", [Host]),
    {ok, [{hook, muc_filter_message, muc_filter_message, 40},
          {hook, user_send_packet, user_send_packet, 40}]}.

stop(_Host) -> ok.
reload(_Host, _NewOpts, _OldOpts) -> ok.
depends(_Host, _Opts) -> [].
mod_options(_Host) -> [].
mod_doc() -> #{desc => [<<"Refuses messages with a body that are not OMEMO encrypted.">>]}.

encrypted(#message{sub_els = Els}) ->
    lists:any(fun(#xmlel{name = <<"encrypted">>} = El) ->
                      lists:member(fxml:get_tag_attr_s(<<"xmlns">>, El), [?NS_OMEMO2, ?NS_OMEMO]);
                 (_) ->
                      false
              end, Els).

has_body(#message{body = Body}) -> Body /= [].

plaintext(Pkt) -> has_body(Pkt) andalso not encrypted(Pkt).

refuse(Pkt) ->
    ?INFO_MSG("Refused a plaintext message from ~ts", [jid:encode(xmpp:get_from(Pkt))]),
    Text = <<"This chat carries only end-to-end encrypted messages. Turn on encryption (the lock) and send again.">>,
    ejabberd_router:route_error(Pkt, xmpp:err_not_acceptable(Text, xmpp:get_lang(Pkt))).

%% A room message, before the room relays or archives it.
muc_filter_message(#message{} = Pkt, _State, _Nick) ->
    case plaintext(Pkt) of
        true -> refuse(Pkt), drop;
        false -> Pkt
    end;
muc_filter_message(Acc, _State, _Nick) ->
    Acc.

%% A member's own message to another member (rooms are checked above).
user_send_packet({#message{type = T} = Pkt, C2SState} = Acc) when T == chat; T == normal ->
    case plaintext(Pkt) of
        true -> refuse(Pkt), {stop, {drop, C2SState}};
        false -> Acc
    end;
user_send_packet(Acc) ->
    Acc.
