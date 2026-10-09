--
-- PostgreSQL database dump — a sample for `harness-view --demo`.
-- pg_dump --schema-only --no-owner
--

SET statement_timeout = 0;
SET client_encoding = 'UTF8';
SELECT pg_catalog.set_config('search_path', '', false);

CREATE TYPE public.class_kind AS ENUM (
    'barbarian',
    'bard',
    'cleric',
    'fighter',
    'rogue',
    'wizard'
);

CREATE TYPE public.session_state AS ENUM (
    'planned',
    'played',
    'cancelled'
);

CREATE FUNCTION public.touch_updated_at() RETURNS trigger
    LANGUAGE plpgsql
    AS $$
BEGIN
  NEW.updated_at = now();
  RETURN NEW;
END;
$$;

CREATE TABLE public.player (
    id uuid DEFAULT gen_random_uuid() NOT NULL,
    email text NOT NULL,
    display_name character varying(60) NOT NULL,
    created_at timestamp with time zone DEFAULT now() NOT NULL,
    CONSTRAINT player_email_check CHECK ((email ~~ '%@%'::text))
);

CREATE TABLE public.campaign (
    id uuid DEFAULT gen_random_uuid() NOT NULL,
    name text NOT NULL,
    game_master_id uuid NOT NULL,
    started_on date,
    archived boolean DEFAULT false NOT NULL,
    updated_at timestamp with time zone DEFAULT now() NOT NULL,
    CONSTRAINT campaign_name_check CHECK ((length(name) > 0))
);

CREATE TABLE public."character" (
    id integer NOT NULL,
    campaign_id uuid NOT NULL,
    player_id uuid NOT NULL,
    name character varying(40) NOT NULL,
    class public.class_kind NOT NULL,
    level smallint DEFAULT 1 NOT NULL,
    hit_points integer NOT NULL,
    max_hit_points integer NOT NULL,
    updated_at timestamp with time zone DEFAULT now() NOT NULL,
    CONSTRAINT character_level_check CHECK (((level >= 1) AND (level <= 20))),
    CONSTRAINT character_hp_check CHECK ((hit_points <= max_hit_points))
);

CREATE SEQUENCE public.character_id_seq
    AS integer
    START WITH 1
    INCREMENT BY 1
    NO MINVALUE
    NO MAXVALUE
    CACHE 1;

ALTER SEQUENCE public.character_id_seq OWNED BY public."character".id;

ALTER TABLE ONLY public."character" ALTER COLUMN id SET DEFAULT nextval('public.character_id_seq'::regclass);

ALTER TABLE ONLY public.player
    ADD CONSTRAINT player_pkey PRIMARY KEY (id);

ALTER TABLE ONLY public.player
    ADD CONSTRAINT player_email_key UNIQUE (email);

ALTER TABLE ONLY public.campaign
    ADD CONSTRAINT campaign_pkey PRIMARY KEY (id);

ALTER TABLE ONLY public."character"
    ADD CONSTRAINT character_pkey PRIMARY KEY (id);

CREATE UNIQUE INDEX character_name_per_campaign ON public."character" USING btree (campaign_id, name);

CREATE TRIGGER campaign_touch BEFORE UPDATE ON public.campaign FOR EACH ROW EXECUTE FUNCTION public.touch_updated_at();

ALTER TABLE ONLY public.campaign
    ADD CONSTRAINT campaign_game_master_id_fkey FOREIGN KEY (game_master_id) REFERENCES public.player(id) ON DELETE RESTRICT;

ALTER TABLE ONLY public."character"
    ADD CONSTRAINT character_campaign_id_fkey FOREIGN KEY (campaign_id) REFERENCES public.campaign(id) ON DELETE CASCADE;

ALTER TABLE ONLY public."character"
    ADD CONSTRAINT character_player_id_fkey FOREIGN KEY (player_id) REFERENCES public.player(id) ON DELETE RESTRICT;
